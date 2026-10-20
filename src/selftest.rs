//! 自检 / 集成测试入口：`MemCleaner.exe --self-test [--out 结果.json]`
//!
//! 用于自动化验证清理效果（尤其是需要管理员权限的系统级清理）：
//! - 非管理员运行：输出每个清理指令的预期错误（STATUS_PRIVILEGE_NOT_HELD），退出码 2；
//! - 管理员运行：执行与 PCL 完全相同的 2→3→4 指令，记录每一步 NTSTATUS、
//!   清理前后可用内存、耗时，结果写 JSON，退出码 0。
//!
//! 该模式不创建窗口、不加载 UI，进程内存占用极低，也可以在计划任务里用来做回归测试。

use crate::config;
use crate::sys;
use serde_json::json;
use std::time::Instant;

/// 自检结果输出结构（写成 JSON，便于自动化读取）。
pub fn run(args: &[String]) -> i32 {
    let out_path = find_out_path(args);
    let start = Instant::now();
    let elevated = sys::is_elevated();
    let before = sys::mem_status();

    let mut privilege_error: Option<u32> = None;
    let mut steps: Vec<serde_json::Value> = Vec::new();
    let mut clean_ok = false;
    let mut error_text: Option<String> = None;
    let mut after = before;
    let mut reclaimed_bytes: u64 = 0;
    let list_before = sys::memory::query_memory_list();
    let mut list_after = list_before;

    if elevated {
        match sys::clean(&sys::CleanOptions::default()) {
            Ok(outcome) => {
                clean_ok = true;
                reclaimed_bytes = outcome.reclaimed_bytes;
                after = sys::MemStatus::from_total_avail(before.total_bytes, outcome.after_avail);
                list_after = sys::memory::query_memory_list();
                for step in &outcome.steps {
                    steps.push(json!({
                        "name": step.name,
                        "success": step.success,
                        "error": step.error_code,
                    }));
                }
                error_text = Some(format!(
                    "freed={} reclaimed={} bytes before={} after={} duration={}ms",
                    outcome.freed_bytes, outcome.reclaimed_bytes, outcome.before_avail, outcome.after_avail, outcome.duration_ms
                ));
            }
            Err(e) => {
                error_text = Some(e.to_string());
            }
        }
    } else {
        match sys::privilege::enable_profile_privilege() {
            Ok(()) => {
                error_text = Some("普通权限下意外启用了 SeProfileSingleProcessPrivilege".to_string());
            }
            Err(code) => {
                privilege_error = Some(code);
                steps.push(json!({
                    "name": "EnableSeProfileSingleProcessPrivilege",
                    "success": false,
                    "error": code,
                }));
            }
        }
        // 直接调用三条指令，记录各自返回的 NTSTATUS（预期全部 STATUS_PRIVILEGE_NOT_HELD）。
        for (cmd, name) in [
            (sys::memory::MEMORY_EMPTY_WORKING_SETS, "EmptyWorkingSets"),
            (sys::memory::MEMORY_FLUSH_MODIFIED_LIST, "FlushModifiedList"),
            (sys::memory::MEMORY_PURGE_STANDBY_LIST, "PurgeStandbyList"),
        ] {
            let report = sys::memory::memory_list_command(cmd, name);
            steps.push(json!({
                "name": report.name,
                "success": report.success,
                "error": report.error_code,
            }));
        }
        error_text = Some(format!("需要管理员权限；特权启用错误码 {:?}", privilege_error));
    }

    let report = json!({
        "tool": "MemCleaner",
        "version": env!("CARGO_PKG_VERSION"),
        "elevated": elevated,
        "os_build": os_build_number(),
        "total_bytes": before.total_bytes,
        "before_avail_bytes": before.avail_bytes,
        "after_avail_bytes": after.avail_bytes,
        "freed_bytes": after.avail_bytes as i64 - before.avail_bytes as i64,
        "reclaimed_bytes": reclaimed_bytes,
        "standby_before_bytes": list_before.map(|l| l.standby_bytes()).unwrap_or(0),
        "standby_after_bytes": list_after.map(|l| l.standby_bytes()).unwrap_or(0),
        "modified_before_bytes": list_before.map(|l| l.modified_bytes()).unwrap_or(0),
        "modified_after_bytes": list_after.map(|l| l.modified_bytes()).unwrap_or(0),
        "clean_success": clean_ok,
        "error": error_text,
        "steps": steps,
        "duration_ms": start.elapsed().as_millis() as u64,
        "timestamp_unix": config::unix_now(),
    });
    let text = serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string());
    if let Some(path) = out_path {
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => println!("self-test result written to {path}"),
            Err(e) => eprintln!("self-test result write FAILED ({path}): {e}"),
        }
    }
    println!("{text}");
    if clean_ok {
        0
    } else if !elevated {
        2
    } else {
        3
    }
}

fn find_out_path(args: &[String]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        if a == "--out" {
            return iter.next().cloned();
        }
        if let Some(rest) = a.strip_prefix("--out=") {
            return Some(rest.to_string());
        }
    }
    None
}

/// 读取 Windows build 号（用于验证报告环境）。
fn os_build_number() -> u32 {
    use windows_sys::Win32::System::Registry::{
        RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, REG_SZ,
    };
    let mut build: u32 = 0;
    unsafe {
        let mut hkey = std::ptr::null_mut();
        let subkey = crate::platform::wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) != 0 {
            return 0;
        }
        let name = crate::platform::wide("CurrentBuildNumber");
        let mut data_type: u32 = 0;
        let mut buf = [0u8; 64];
        let mut size: u32 = buf.len() as u32;
        let result = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut data_type as *mut u32,
            buf.as_mut_ptr(),
            &mut size,
        );
        let _ = windows_sys::Win32::System::Registry::RegCloseKey(hkey);
        if result == 0 {
            let len = (size as usize / 2).saturating_sub(1).min(buf.len() / 2);
            let utf16: Vec<u16> = buf[..len * 2]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if data_type == REG_SZ {
                if let Ok(s) = String::from_utf16(&utf16) {
                    build = s.trim().parse().unwrap_or(0);
                }
            }
        }
    }
    build
}
