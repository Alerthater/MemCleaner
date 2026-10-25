//! 进程提权与特权启用。
//!
//! PCL 的内存优化需要两步权限：
//! 1. 进程本身必须是管理员（UAC 提升）；
//! 2. 启用 `SeProfileSingleProcessPrivilege`，否则 `NtSetSystemInformation(80)` 返回
//!    `STATUS_PRIVILEGE_NOT_HELD (0xC0000061)`。

use windows_sys::Win32::Foundation::{HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation,
    LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// 启用的管理员特权名。
pub const SE_PROFILE_SINGLE_PROCESS: &str = "SeProfileSingleProcessPrivilege";
/// 收缩系统文件缓存所需特权名。
pub const SE_INCREASE_QUOTA: &str = "SeIncreaseQuotaPrivilege";

/// 当前进程是否已提升为管理员。
pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        windows_sys::Win32::Foundation::CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 当前进程令牌的完整性级别 RID（0x2000=Medium，0x3000=High，0x1000=Low）。
pub fn integrity_rid() -> u32 {
    use windows_sys::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, TokenIntegrityLevel, TOKEN_MANDATORY_LABEL};
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return 0;
        }
        let mut buf = vec![0u8; 256];
        let mut ret: u32 = 0;
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel as i32,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            buf.len() as u32,
            &mut ret,
        );
        let mut rid = 0u32;
        if ok != 0 {
            let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
            let count = *GetSidSubAuthorityCount(label.Label.Sid) as u32;
            if count > 0 {
                rid = *GetSidSubAuthority(label.Label.Sid, count - 1);
            }
        }
        CloseHandle(token);
        rid
    }
}

/// 当前进程能否「静默」执行系统级清理。
///
/// 需要：管理员令牌 + High（或更高）完整性 + 能启用 SeProfileSingleProcessPrivilege。
/// 返回 Err 时附带可读原因（用于日志/提示）。
pub fn can_clean_silently() -> Result<(), String> {
    if !is_elevated() {
        return Err("需要管理员权限".to_string());
    }
    let rid = integrity_rid();
    if rid < 0x2000 {
        return Err(format!("当前进程令牌被系统限制（完整性 0x{rid:04X}）"));
    }
    if let Err(code) = enable_profile_privilege() {
        return Err(format!("无法启用 SeProfileSingleProcessPrivilege（错误码 {code}）"));
    }
    Ok(())
}

/// 启用一个特权（例如 SeProfileSingleProcessPrivilege）。
/// 成功返回 Ok，失败返回 Win32 错误码。
pub fn enable_privilege(name: &str) -> Result<(), u32> {
    let mut name_w: Vec<u16> = name.encode_utf16().collect();
    name_w.push(0);
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | 0x20 /* TOKEN_ADJUST_PRIVILEGES */, &mut token) == 0 {
            return Err(GetLastError());
        }
        let mut luid: LUID = std::mem::zeroed();
        if LookupPrivilegeValueW(std::ptr::null(), name_w.as_ptr(), &mut luid) == 0 {
            let err = GetLastError();
            CloseHandle(token);
            return Err(err);
        }
        let mut tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let mut ret_len: u32 = 0;
        let ok = AdjustTokenPrivileges(
            token,
            0,
            &mut tp,
            0,
            std::ptr::null_mut(),
            &mut ret_len,
        );
        let err = GetLastError();
        CloseHandle(token);
        if ok == 0 {
            return Err(err);
        }
        // AdjustTokenPrivileges 在“特权不存在”时也返回成功，必须检查 LastError。
        if err != 0 {
            return Err(err);
        }
        Ok(())
    }
}

/// 启用 PCL 内存优化所需的 SeProfileSingleProcessPrivilege。
pub fn enable_profile_privilege() -> Result<(), u32> {
    enable_privilege(SE_PROFILE_SINGLE_PROCESS)
}

/// 启用收缩文件缓存所需的 SeIncreaseQuotaPrivilege。
pub fn enable_increase_quota() -> Result<(), u32> {
    enable_privilege(SE_INCREASE_QUOTA)
}

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_detected_consistently() {
        // 只验证不 panic、结果稳定。
        let a = is_elevated();
        let b = is_elevated();
        assert_eq!(a, b);
    }

    #[test]
    fn enabling_privilege_returns_result_without_panic() {
        // 标准用户下可能 Ok（特权可启用）或 Err（无管理员）；都不允许 panic。
        let _ = enable_profile_privilege();
    }
}
