//! 物理内存状态读取与 `NtSetSystemInformation` 清理指令。
//!
//! 行为参考 PCL 2.13.1.1 的「内存优化」（仅参考可观察行为，未复制其代码，见 NOTICE.md）：
//! ```text
//! for num = 2 To 4
//!     NtSetSystemInformation(80, &num, 4)
//! Next
//! ```
//! 80 = SystemMemoryListInformation（SYSTEM_INFORMATION_CLASS），
//! 参数为 MEMORY_LIST_COMMAND：2=MemoryEmptyWorkingSets、3=MemoryFlushModifiedList、
//! 4=MemoryPurgeStandbyList、5=MemoryPurgeLowPriorityStandbyList。

use super::{CleanOutcome, CleanError, StepReport};

/// 系统内存列表统计（来自 NtQuerySystemInformation(SystemMemoryListInformation=80)）。
///
/// 注意：Windows 的「可用内存」(ullAvailPhys) **包含** standby 缓存，所以只清除
/// standby 时可用内存几乎不变——这会让清理效果被低估。这里直接读列表量，
/// 用 standby/modified 的下降量衡量真实清理效果（与 Mem Reduct 口径一致）。
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryListInfo {
    /// 待机列表总页数（所有优先级之和）。
    pub standby_pages: u64,
    /// 已修改页面数。
    pub modified_pages: u64,
    /// 空闲页面数。
    pub free_pages: u64,
    /// 零页数。
    pub zero_pages: u64,
}

impl MemoryListInfo {
    /// 待机 + 已修改页面的字节数（可回收缓存）。
    pub fn reclaimable_bytes(&self) -> u64 {
        (self.standby_pages + self.modified_pages) * PAGE_SIZE
    }
    pub fn standby_bytes(&self) -> u64 {
        self.standby_pages * PAGE_SIZE
    }
    pub fn modified_bytes(&self) -> u64 {
        self.modified_pages * PAGE_SIZE
    }
}

/// 页大小（Windows x64 固定 4 KiB）。
const PAGE_SIZE: u64 = 4096;

/// SYSTEM_MEMORY_LIST_INFORMATION 的页计数字段个数（Zero/Free/Modified/ModifiedNoWrite/Bad + 8 优先级 + 8 复用 + ModifiedPageFile）。
const MEMORY_LIST_PAGE_FIELDS: usize = 5 + 8 + 8 + 1;
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

/// 内存清理指令：清空所有进程工作集（PCL 第一步）。
pub const MEMORY_EMPTY_WORKING_SETS: u32 = 2;
/// 内存清理指令：刷新已修改页面列表（PCL 第二步）。
pub const MEMORY_FLUSH_MODIFIED_LIST: u32 = 3;
/// 内存清理指令：清除待机列表（PCL 第三步）。
pub const MEMORY_PURGE_STANDBY_LIST: u32 = 4;
/// 内存清理指令：只清除低优先级待机列表（Mem Reduct 的轻量待机清理）。
pub const MEMORY_PURGE_LOW_PRIORITY_STANDBY_LIST: u32 = 5;

/// SystemMemoryListInformation
const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;

/// 读取物理内存状态（GlobalMemoryStatusEx）。
pub fn mem_status() -> super::MemStatus {
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut m) };
    if ok == 0 {
        return super::MemStatus::default();
    }
    super::MemStatus::from_total_avail(m.ullTotalPhys, m.ullAvailPhys)
}

/// 执行一条 MEMORY_LIST_COMMAND。
///
/// 返回 `StepReport`（失败不 panic，把 NTSTATUS 带回去）。
pub fn memory_list_command(command: u32, name: &'static str) -> StepReport {
    let mut value = command;
    let status = unsafe {
        NtSetSystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            &mut value as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    StepReport {
        name,
        success: status == 0,
        error_code: if status == 0 { None } else { Some(status) },
    }
}

/// 收缩系统文件缓存（SetSystemFileCacheSize(-1, -1, 0)），效果等同 Mem Reduct 的
/// “系统文件缓存”清理项。需要 SeIncreaseQuotaPrivilege。
pub fn trim_system_file_cache() -> StepReport {
    let ok = unsafe { SetSystemFileCacheSize(usize::MAX, usize::MAX, 0) };
    StepReport {
        name: "TrimFileCache",
        success: ok != 0,
        error_code: if ok != 0 { None } else { Some(unsafe { GetLastError() }) },
    }
}

/// 查询系统内存列表（standby/modified/free/zero 页数）。普通权限即可查询。
pub fn query_memory_list() -> Option<MemoryListInfo> {
    #[repr(C)]
    struct MemoryList {
        counts: [usize; MEMORY_LIST_PAGE_FIELDS],
    }
    let mut list = MemoryList {
        counts: [0; MEMORY_LIST_PAGE_FIELDS],
    };
    let mut ret_len: u32 = 0;
    let status = unsafe {
        NtQuerySystemInformation(
            SYSTEM_MEMORY_LIST_INFORMATION,
            &mut list as *mut MemoryList as *mut core::ffi::c_void,
            std::mem::size_of::<MemoryList>() as u32,
            &mut ret_len,
        )
    };
    if status != 0 {
        return None;
    }
    let c = &list.counts;
    // 字段顺序：ZeroPageCount, FreePageCount, ModifiedPageCount, ModifiedNoWritePageCount,
    // BadPageCount, PageCountByPriority[8], RepurposedPagesByPriority[8], ModifiedPageCountPageFile
    let standby: u64 = c[5..13].iter().map(|v| *v as u64).sum();
    Some(MemoryListInfo {
        zero_pages: c[0] as u64,
        free_pages: c[1] as u64,
        modified_pages: c[2] as u64,
        standby_pages: standby,
    })
}

/// 读取当前可用内存（PCL 用 ComputerInfo.AvailablePhysicalMemory，底层同为 GlobalMemoryStatusEx）。
pub fn avail_bytes() -> u64 {
    mem_status().avail_bytes
}

/// 便捷包装：读 before/after 并执行 PCL 的三条指令（供自检工具使用）。
pub fn clean_pcl_equivalent() -> Result<CleanOutcome, CleanError> {
    crate::engine::execute_clean(&super::CleanOptions::default())
}

use windows_sys::Win32::Foundation::GetLastError;

#[link(name = "ntdll")]
extern "system" {
    /// `NtSetSystemInformation`，PCL 直接用 ntdll.dll 的导出。
    fn NtSetSystemInformation(
        system_information_class: u32,
        system_information: *mut core::ffi::c_void,
        system_information_length: u32,
    ) -> u32;
}

#[link(name = "ntdll")]
extern "system" {
    /// `NtQuerySystemInformation`，读取系统内存列表用。
    fn NtQuerySystemInformation(
        system_information_class: u32,
        system_information: *mut core::ffi::c_void,
        system_information_length: u32,
        return_length: *mut u32,
    ) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    /// `SetSystemFileCacheSize`，kernel32.dll 导出（需 SeIncreaseQuotaPrivilege）。
    fn SetSystemFileCacheSize(
        minimum_file_cache_size: usize,
        maximum_file_cache_size: usize,
        flags: u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_pcl() {
        assert_eq!(MEMORY_EMPTY_WORKING_SETS, 2);
        assert_eq!(MEMORY_FLUSH_MODIFIED_LIST, 3);
        assert_eq!(MEMORY_PURGE_STANDBY_LIST, 4);
        assert_eq!(SYSTEM_MEMORY_LIST_INFORMATION, 80);
    }

    #[test]
    fn mem_status_is_sane() {
        let s = mem_status();
        // 任何正常 PC 都有 > 1GB 内存。
        if s.total_bytes > 0 {
            assert!(s.total_bytes > 1024 * 1024 * 1024);
            assert!(s.avail_bytes <= s.total_bytes);
            assert!(s.percent <= 100);
        }
    }
}
