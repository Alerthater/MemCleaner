//! 系统底层能力：内存状态、特权、清理指令。
//!
//! ## 接口契约（由 engine-dev 实现，其他模块只依赖这里的公开 API）
//!
//! - [`mem_status`]：读取物理内存状态（GlobalMemoryStatusEx），每次调用约 1µs。
//! - [`is_elevated`]：当前进程是否具备管理员完整性（UAC 已提升）。
//! - [`clean`]：执行一次完整清理，**会阻塞 0.2~2 秒**，必须在工作线程调用。
//!
//! 清理效果与 PCL 启动器 2.13.1.1 的「内存优化」完全一致：
//! 1. 启用 `SeProfileSingleProcessPrivilege`；
//! 2. 按 2 → 3 → 4 的顺序调用
//!    `NtSetSystemInformation(SystemMemoryListInformation=80, &cmd, 4)`，
//!    即 MemoryEmptyWorkingSets / MemoryFlushModifiedList / MemoryPurgeStandbyList；
//! 3. 进阶项：5 = MemoryPurgeLowPriorityStandbyList、SetSystemFileCacheSize(-1) 收缩文件缓存。
//!
//! 失败时返回 [`CleanError`]，由调用方决定如何提示；任何一步失败都不 panic。

pub mod memory;
pub mod privilege;

use std::fmt;

/// 物理内存状态快照。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemStatus {
    /// 物理内存总量（字节）。
    pub total_bytes: u64,
    /// 当前可用物理内存（字节），对应 `MEMORYSTATUSEX.ullAvailPhys`。
    pub avail_bytes: u64,
    /// 已用物理内存（字节）。
    pub used_bytes: u64,
    /// 使用率百分比 0..=100。
    pub percent: u8,
}

impl MemStatus {
    /// 用 total/avail 计算派生字段。
    pub fn from_total_avail(total_bytes: u64, avail_bytes: u64) -> Self {
        let used = total_bytes.saturating_sub(avail_bytes);
        let percent = if total_bytes == 0 {
            0
        } else {
            ((used as f64 / total_bytes as f64) * 100.0).round().clamp(0.0, 100.0) as u8
        };
        MemStatus {
            total_bytes,
            avail_bytes,
            used_bytes: used,
            percent,
        }
    }
}

/// 一次清理要执行哪些系统指令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanOptions {
    /// 2 = MemoryEmptyWorkingSets（PCL 第一步，效果最明显）。
    pub empty_working_sets: bool,
    /// 3 = MemoryFlushModifiedList。
    pub flush_modified: bool,
    /// 4 = MemoryPurgeStandbyList。
    pub purge_standby: bool,
    /// 5 = MemoryPurgeLowPriorityStandbyList（进阶）。
    pub purge_low_priority: bool,
    /// SetSystemFileCacheSize(-1) 收缩系统文件缓存（进阶，需 SeIncreaseQuotaPrivilege）。
    pub trim_file_cache: bool,
}

impl Default for CleanOptions {
    fn default() -> Self {
        // 默认 = PCL 启动器的三条指令。
        CleanOptions {
            empty_working_sets: true,
            flush_modified: true,
            purge_standby: true,
            purge_low_priority: false,
            trim_file_cache: false,
        }
    }
}

/// 单步执行结果（用于日志与调试展示）。
#[derive(Clone, Debug)]
pub struct StepReport {
    /// 指令名，如 "EmptyWorkingSets"。
    pub name: &'static str,
    /// 是否执行成功。
    pub success: bool,
    /// 失败时的 NTSTATUS 或 Win32 错误码。
    pub error_code: Option<u32>,
}

/// 一次清理的完整结果。
#[derive(Clone, Debug, Default)]
pub struct CleanOutcome {
    /// 清理前可用物理内存。
    pub before_avail: u64,
    /// 清理后可用物理内存。
    pub after_avail: u64,
    /// 可用物理内存变化量（字节，可为负；standby 缓存被清不影响 ullAvailPhys，因此可能偏小）。
    pub freed_bytes: i64,
    /// 实际回收的缓存量（standby + modified 下降量，字节）。这是更接近真实清理效果的指标。
    pub reclaimed_bytes: u64,
    /// 逐步执行结果。
    pub steps: Vec<StepReport>,
    /// 总耗时（毫秒）。
    pub duration_ms: u64,
}

/// 清理失败原因。
#[derive(Clone, Debug)]
pub enum CleanError {
    /// 没有管理员权限（需要以管理员身份重启）。
    NotElevated,
    /// 启用某个特权失败。
    Privilege { name: &'static str, code: u32 },
    /// 系统指令执行失败。
    Step { name: &'static str, code: u32 },
    /// 其他底层错误（如内存状态读取失败）。
    Os(String),
}

impl fmt::Display for CleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CleanError::NotElevated => write!(f, "需要管理员权限"),
            CleanError::Privilege { name, code } => write!(f, "启用特权 {name} 失败（错误码 {code}）"),
            CleanError::Step { name, code } => write!(f, "清理指令 {name} 失败（错误码 {code}）"),
            CleanError::Os(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CleanError {}

/// 读取当前物理内存状态。
pub fn mem_status() -> MemStatus {
    memory::mem_status()
}

/// 当前进程是否已提升为管理员。
pub fn is_elevated() -> bool {
    privilege::is_elevated()
}

/// 执行一次清理（阻塞，须在工作线程调用）。
pub fn clean(opts: &CleanOptions) -> Result<CleanOutcome, CleanError> {
    engine_clean(opts)
}

/// 实际清理实现：让 sys 保持薄，逻辑集中在 engine 模块。
/// 这里仅作为 sys::clean 的转入口，避免循环依赖。
fn engine_clean(opts: &CleanOptions) -> Result<CleanOutcome, CleanError> {
    crate::engine::execute_clean(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_math() {
        let s = MemStatus::from_total_avail(16 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
        assert_eq!(s.percent, 50);
        assert_eq!(s.used_bytes, 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn default_opts_match_pcl() {
        let o = CleanOptions::default();
        assert!(o.empty_working_sets && o.flush_modified && o.purge_standby);
        assert!(!o.purge_low_priority && !o.trim_file_cache);
    }
}
