//! 运行环境诊断：把「进程处于什么上下文」写进日志，用于排查开机自启/清理失败。
//!
//! 记录内容：会话 ID、用户 SID、提权状态、完整性级别、窗口站/桌面、关键特权是否存在/已启用、
//! 关键目录是否可写（含错误码）。全部失败都静默降级，不会影响程序正常运行。

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, LookupPrivilegeValueW,
    TokenElevation, TokenIntegrityLevel, TokenPrivileges, TokenUser,
    LUID_AND_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, HDESK, HWINSTA, UOI_NAME,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThreadId, OpenProcessToken};

fn last_err() -> String {
    let code = unsafe { GetLastError() };
    format!("win32={code}")
}

fn wstr_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 从令牌里取 SID 的字符串形式。
fn token_sid_string(token: HANDLE) -> String {
    let mut buf = vec![0u8; 256];
    let mut ret = 0u32;
    unsafe {
        if GetTokenInformation(
            token,
            TokenUser as TOKEN_INFORMATION_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut ret,
        ) == 0
        {
            return format!("<TokenUser 失败 {}>", last_err());
        }
        // TOKEN_USER 的第一个字段就是 SID_AND_ATTRIBUTES
        let sid = *(buf.as_ptr() as *const *mut c_void);
        let mut out: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut out) == 0 {
            return format!("<ConvertSidToStringSidW 失败 {}>", last_err());
        }
        let mut len = 0usize;
        while *out.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
        LocalFree(out as HLOCAL);
        s
    }
}

/// 从令牌里取完整性级别 RID 并翻译（0x1000=Medium，0x2000=High，0x3000=System…）。
fn token_integrity(token: HANDLE) -> String {
    let mut buf = vec![0u8; 256];
    let mut ret = 0u32;
    unsafe {
        if GetTokenInformation(
            token,
            TokenIntegrityLevel as TOKEN_INFORMATION_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut ret,
        ) == 0
        {
            return format!("<TokenIntegrityLevel 失败 {}>", last_err());
        }
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let count = *GetSidSubAuthorityCount(label.Label.Sid) as u32;
        if count == 0 {
            return "<empty SID>".to_string();
        }
        let rid = *GetSidSubAuthority(label.Label.Sid, count - 1);
        let name = match rid {
            0x0000 => "Untrusted",
            0x1000 => "Low",
            0x2000 => "Medium",
            0x3000 => "High",
            0x4000 => "System",
            _ => "Custom",
        };
        format!("{name}(0x{rid:04X})")
    }
}

/// 查询某个特权在令牌里是否存在、是否已启用。
fn privilege_state(token: HANDLE, name: &str) -> String {
    let mut luid = windows_sys::Win32::Foundation::LUID {
        LowPart: 0,
        HighPart: 0,
    };
    let mut name_w = wstr_z(name);
    unsafe {
        if LookupPrivilegeValueW(std::ptr::null(), name_w.as_mut_ptr(), &mut luid) == 0 {
            return format!("{name}=<不存在/查询失败 {}>", last_err());
        }
        // 取足够大的缓冲区读取全部特权。
        let mut buf = vec![0u8; 4096];
        let mut ret = 0u32;
        if GetTokenInformation(
            token,
            TokenPrivileges as TOKEN_INFORMATION_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            buf.len() as u32,
            &mut ret,
        ) == 0
        {
            return format!("{name}=<TokenPrivileges 失败 {}>", last_err());
        }
        let tp = &*(buf.as_ptr() as *const TOKEN_PRIVILEGES);
        let count = tp.PrivilegeCount as usize;
        let arr = std::slice::from_raw_parts(tp.Privileges.as_ptr(), count);
        let found = arr.iter().find(|p: &&LUID_AND_ATTRIBUTES| {
            p.Luid.LowPart == luid.LowPart && p.Luid.HighPart == luid.HighPart
        });
        match found {
            Some(p) => {
                let enabled = (p.Attributes & 0x2) != 0; // SE_PRIVILEGE_ENABLED
                format!("{name}={}", if enabled { "已启用" } else { "存在未启用" })
            }
            None => format!("{name}=<不存在>"),
        }
    }
}

/// 取窗口站/桌面名称（用 / 分隔，失败时返回错误说明）。
fn station_desktop() -> String {
    unsafe fn obj_name(h: HANDLE) -> String {
        if h.is_null() {
            return "<null>".to_string();
        }
        let mut buf = [0u16; 256];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(
            h,
            UOI_NAME,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 2) as u32,
            &mut needed,
        );
        if ok == 0 {
            return format!("<查询失败 {}>", last_err());
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }
    unsafe {
        let winsta: HWINSTA = GetProcessWindowStation();
        let desk: HDESK = GetThreadDesktop(GetCurrentThreadId());
        format!("{}/{}", obj_name(winsta), obj_name(desk))
    }
}

/// 写测试：尝试在目录下创建临时文件；成功返回 `OK`，失败返回错误信息。
fn write_test(dir: &std::path::Path) -> String {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("diag-write-test.tmp");
    match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&path);
            "OK".to_string()
        }
        Err(e) => format!("FAIL({e})"),
    }
}

/// 生成多行诊断信息（每行建议单独写入日志）。
pub fn environment_report() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut session = 0u32;
        let _ = ProcessIdToSessionId(
            windows_sys::Win32::System::Threading::GetCurrentProcessId(),
            &mut session,
        );
        out.push(format!("diag session={session}"));

        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            out.push(format!("diag token=<OpenProcessToken 失败 {}>", last_err()));
            return out;
        }
        out.push(format!("diag user_sid={}", token_sid_string(token)));
        out.push(format!("diag integrity={}", token_integrity(token)));

        let mut elev: TOKEN_ELEVATION = std::mem::zeroed();
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation as TOKEN_INFORMATION_CLASS,
            &mut elev as *mut TOKEN_ELEVATION as *mut c_void,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        out.push(format!(
            "diag elevated={}",
            if ok != 0 { elev.TokenIsElevated != 0 } else { false }
        ));
        out.push(format!(
            "diag privilege {} {}",
            privilege_state(token, "SeProfileSingleProcessPrivilege"),
            privilege_state(token, "SeIncreaseQuotaPrivilege")
        ));
        CloseHandle(token);
    }
    out.push(format!("diag station_desktop={}", station_desktop()));
    out.push(format!(
        "diag localappdata={:?} temp={:?}",
        std::env::var_os("LOCALAPPDATA").map(|v| v.to_string_lossy().to_string()),
        std::env::var_os("TEMP").map(|v| v.to_string_lossy().to_string())
    ));
    if let Some(dir) = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from) {
        out.push(format!(
            "diag write_profile={}",
            write_test(&dir.join("MemCleaner"))
        ));
    }
    out.push(format!(
        "diag write_temp={}",
        write_test(&std::env::temp_dir().join("MemCleaner"))
    ));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.push(format!("diag write_exedir={}", write_test(dir)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_has_expected_lines() {
        let report = environment_report().join("\n");
        assert!(report.contains("diag session="));
        assert!(report.contains("diag integrity="));
        assert!(report.contains("diag elevated="));
        assert!(report.contains("SeProfileSingleProcessPrivilege"));
    }

    #[test]
    fn integrity_label_formats() {
        assert_eq!(format!("{}(0x{:04X})", "High", 0x2000u32), "High(0x2000)");
    }
}
