//! 内存清理引擎：把配置翻译成系统指令，并负责「测量 → 清理 → 再测量」的完整流程。
//!
//! 与 PCL 启动器 2.13.1.1 的对应关系：
//! - PCL 在管理员进程内直接执行三条 `NtSetSystemInformation`；
//!   非管理员时 `runas --memory` 子进程执行，并通过退出码回传释放的 KB。
//! - 本程序选择「以管理员身份重启自身」的方式（见 platform::elevate），
//!   因此清理本身总是在已提权进程里同步执行。
//!
//! 注意：`execute_clean` 会阻塞（0.2~2s），**只能在工作线程调用**。

use crate::config::Config;
use crate::sys::{self, CleanError, CleanOptions, CleanOutcome, MemStatus};
use std::time::Instant;

/// 把界面配置翻译成系统清理选项。
pub fn options_from_config(cfg: &Config) -> CleanOptions {
    CleanOptions {
        empty_working_sets: cfg.clean_working_sets,
        flush_modified: cfg.clean_modified,
        purge_standby: cfg.clean_standby,
        purge_low_priority: cfg.clean_low_priority,
        trim_file_cache: cfg.trim_file_cache,
    }
}

/// 执行一次清理。**阻塞调用，请放在工作线程里。**
///
/// 顺序与 PCL 一致：先测可用内存 → 启用特权 → 2/3/4 →（可选 5、文件缓存）→ 再测可用内存。
pub fn execute_clean(opts: &CleanOptions) -> Result<CleanOutcome, CleanError> {
    let start = Instant::now();
    if !sys::is_elevated() {
        // PCL 同样先要求管理员权限，否则启用 SeProfileSingleProcessPrivilege 会失败。
        return Err(CleanError::NotElevated);
    }
    let before = sys::mem_status();
    if before.total_bytes == 0 {
        return Err(CleanError::Os("无法读取物理内存状态".to_string()));
    }
    // standby/modified 缓存量：可用内存口径含 standby，单看 ullAvailPhys 会低估清理效果。
    let list_before = sys::memory::query_memory_list();
    let mut steps = Vec::with_capacity(5);

    if let Err(code) = sys::privilege::enable_profile_privilege() {
        crate::app::diag_log(&format!(
            "clean FAILED: cannot enable SeProfileSingleProcessPrivilege (code={code})"
        ));
        return Err(CleanError::Privilege {
            name: "SeProfileSingleProcessPrivilege",
            code,
        });
    }

    // 与 PCL 完全相同的三条指令，顺序 2 → 3 → 4。
    if opts.empty_working_sets {
        steps.push(sys::memory::memory_list_command(
            sys::memory::MEMORY_EMPTY_WORKING_SETS,
            "EmptyWorkingSets",
        ));
    }
    if opts.flush_modified {
        steps.push(sys::memory::memory_list_command(
            sys::memory::MEMORY_FLUSH_MODIFIED_LIST,
            "FlushModifiedList",
        ));
    }
    if opts.purge_standby {
        steps.push(sys::memory::memory_list_command(
            sys::memory::MEMORY_PURGE_STANDBY_LIST,
            "PurgeStandbyList",
        ));
    }

    // ---------- 进阶项（默认关闭）----------
    if opts.purge_low_priority {
        steps.push(sys::memory::memory_list_command(
            sys::memory::MEMORY_PURGE_LOW_PRIORITY_STANDBY_LIST,
            "PurgeLowPriorityStandbyList",
        ));
    }
    if opts.trim_file_cache {
        // 修复：SetSystemFileCacheSize(-1,-1,0) 需要 SeIncreaseQuotaPrivilege，
        // 之前没有启用该特权导致勾选「收缩文件缓存」后整次清理以
        // ERROR_PRIVILEGE_NOT_HELD(1314) 失败。这里先启用特权再收缩。
        match sys::privilege::enable_increase_quota() {
            Ok(()) => steps.push(sys::memory::trim_system_file_cache()),
            Err(code) => steps.push(sys::StepReport {
                name: "EnableSeIncreaseQuotaPrivilege",
                success: false,
                error_code: Some(code),
            }),
        }
    }

    // 任一必选步骤失败 → 报错（PCL 同样会在失败时抛出并提示）。
    if let Some(failed) = steps.iter().find(|s| !s.success) {
        for s in &steps {
            crate::app::diag_log(&format!(
                "clean step {} ok={} err={:?}",
                s.name, s.success, s.error_code
            ));
        }
        return Err(CleanError::Step {
            name: failed.name,
            code: failed.error_code.unwrap_or(0),
        });
    }

    let after = sys::mem_status();
    let freed_bytes = after.avail_bytes as i64 - before.avail_bytes as i64;
    let list_after = sys::memory::query_memory_list();
    let reclaimed_bytes = match (list_before, list_after) {
        (Some(b), Some(a)) => {
            let dropped = b.reclaimable_bytes().saturating_sub(a.reclaimable_bytes());
            // 以 avail 增量兜底：大幅释放工作集时也可能大于缓存下降量。
            dropped.max(freed_bytes.max(0) as u64)
        }
        _ => freed_bytes.max(0) as u64,
    };
    Ok(CleanOutcome {
        before_avail: before.avail_bytes,
        after_avail: after.avail_bytes,
        freed_bytes,
        reclaimed_bytes,
        steps,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// 自动清理判定：纯函数，便于单元测试。
///
/// - `status`：当前内存状态；
/// - `secs_since_last_clean`：距离上次清理（任意来源）的秒数，从未清理为 None；
/// - 返回 true 时调用方应启动一次清理。
pub fn should_auto_clean(cfg: &Config, status: &MemStatus, secs_since_last_clean: Option<u64>) -> bool {
    if !cfg.auto_clean_configured() {
        return false;
    }
    let since = secs_since_last_clean.unwrap_or(u64::MAX);
    if since < cfg.auto_cooldown_seconds as u64 {
        return false;
    }
    let threshold_hit = cfg.auto_threshold_percent > 0 && status.percent >= cfg.auto_threshold_percent;
    let interval_hit = cfg.auto_interval_minutes > 0
        && since >= cfg.auto_interval_minutes as u64 * 60
        // 固定间隔清理只在内存确实偏满时才有意义，避免空转。
        && status.percent >= 50;
    threshold_hit || interval_hit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::MemStatus;

    fn status(p: u8) -> MemStatus {
        MemStatus {
            total_bytes: 100,
            avail_bytes: 100 - p as u64,
            used_bytes: p as u64,
            percent: p,
        }
    }

    #[test]
    fn options_follow_config() {
        let mut cfg = Config::default();
        cfg.clean_standby = false;
        cfg.trim_file_cache = true;
        let o = options_from_config(&cfg);
        assert!(o.empty_working_sets);
        assert!(!o.purge_standby);
        assert!(o.trim_file_cache);
    }

    #[test]
    fn auto_clean_respects_cooldown_and_threshold() {
        let mut cfg = Config::default();
        cfg.auto_clean_enabled = true;
        cfg.auto_threshold_percent = 80;
        cfg.auto_cooldown_seconds = 60;
        assert!(!should_auto_clean(&cfg, &status(50), Some(999)));
        assert!(should_auto_clean(&cfg, &status(85), Some(61)));
        assert!(!should_auto_clean(&cfg, &status(85), Some(10)));
        // 从未清理过（None）视作冷却已满足：首次达到阈值即允许清理
        assert!(should_auto_clean(&cfg, &status(85), None));
        // 未启用
        cfg.auto_clean_enabled = false;
        assert!(!should_auto_clean(&cfg, &status(99), Some(999)));
    }

    #[test]
    fn interval_trigger_requires_half_memory() {
        let mut cfg = Config::default();
        cfg.auto_clean_enabled = true;
        cfg.auto_threshold_percent = 0;
        cfg.auto_interval_minutes = 1;
        cfg.auto_cooldown_seconds = 0;
        assert!(!should_auto_clean(&cfg, &status(40), Some(120)));
        assert!(should_auto_clean(&cfg, &status(70), Some(120)));
    }
}
