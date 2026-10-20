//! 配置读写。配置文件位于 %LOCALAPPDATA%\MemCleaner\config.json，原子写入。
//!
//! 设计目标：字段少、解析零压力、损坏时自动回退默认值（不 panic、不弹窗）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// 主题模式。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// 跟随系统（读取注册表 AppsUseLightTheme）。
    System,
    Dark,
    Light,
}

impl Default for ThemeMode {
    fn default() -> Self {
        ThemeMode::System
    }
}

/// 应用配置（全部字段带默认值，旧版本配置缺字段也能读）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    // ---------- 自动清理 ----------
    /// 是否启用自动清理（需要管理员权限）。
    pub auto_clean_enabled: bool,
    /// 内存使用率高于该百分比时自动清理；0 表示不按阈值触发。
    pub auto_threshold_percent: u8,
    /// 固定间隔自动清理（分钟）；0 表示关闭。
    pub auto_interval_minutes: u32,
    /// 两次自动清理之间的最小冷却时间（秒）。
    pub auto_cooldown_seconds: u32,

    // ---------- 清理项（默认等同于 PCL 启动器的三条指令）----------
    /// MemoryEmptyWorkingSets：清空所有进程工作集。
    pub clean_working_sets: bool,
    /// MemoryFlushModifiedList：刷新已修改页面列表。
    pub clean_modified: bool,
    /// MemoryPurgeStandbyList：清除待机列表。
    pub clean_standby: bool,
    /// MemoryPurgeLowPriorityStandbyList：清除低优先级待机列表（进阶）。
    pub clean_low_priority: bool,
    /// SetSystemFileCacheSize(-1)：收缩系统文件缓存（进阶，Mem Reduct 同款）。
    pub trim_file_cache: bool,

    // ---------- 界面 ----------
    pub theme: ThemeMode,
    /// 关闭窗口时最小化到托盘而不是退出。
    pub minimize_to_tray: bool,
    /// 是否播放动画（低性能设备可关闭）。
    pub animations: bool,

    // ---------- 系统 ----------
    /// 是否已请求开机自启（计划任务，管理员权限，静默）。
    pub start_with_windows: bool,

    // ---------- 历史结果 ----------
    /// 上次清理释放的字节数。
    pub last_freed_bytes: u64,
    /// 上次清理完成时间（Unix 秒）。
    pub last_clean_unix: u64,
    /// 累计清理次数。
    pub clean_count: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            auto_clean_enabled: false,
            auto_threshold_percent: 85,
            auto_interval_minutes: 0,
            auto_cooldown_seconds: 60,
            clean_working_sets: true,
            clean_modified: true,
            clean_standby: true,
            clean_low_priority: false,
            trim_file_cache: false,
            theme: ThemeMode::System,
            minimize_to_tray: true,
            animations: true,
            start_with_windows: false,
            last_freed_bytes: 0,
            last_clean_unix: 0,
            clean_count: 0,
        }
    }
}

impl Config {
    /// 范围校验：把越界/不合理的值钳制或回退默认值（QA Bug-02）。
    /// 在 load() 解析后、save() 写入前都调用一次，保证配置文件永远合法。
    pub fn sanitize(&mut self) {
        // 阈值：允许 0（关闭阈值触发）或 50..=100，其它一律回退 85。
        self.auto_threshold_percent = match self.auto_threshold_percent {
            0 | 50..=100 => self.auto_threshold_percent,
            _ => 85,
        };
        // 固定间隔：0..=1440 分钟（一天），越界回退 0（关闭）。
        if self.auto_interval_minutes > 1440 {
            self.auto_interval_minutes = 0;
        }
        // 冷却时间：10..=3600 秒，越界回退 60。
        if !(10..=3600).contains(&self.auto_cooldown_seconds) {
            self.auto_cooldown_seconds = 60;
        }
        // 历史字段做上限保护，避免异常大数影响格式化。
        if self.last_freed_bytes > 1 << 50 {
            self.last_freed_bytes = 0;
        }
    }

    /// 自动清理是否具备基本条件（启用且至少配置了一种触发方式）。
    pub fn auto_clean_configured(&self) -> bool {
        self.auto_clean_enabled
            && (self.auto_threshold_percent > 0 || self.auto_interval_minutes > 0)
    }

    /// 本轮清理是否有任何清理项被选中。
    pub fn has_any_item(&self) -> bool {
        self.clean_working_sets
            || self.clean_modified
            || self.clean_standby
            || self.clean_low_priority
            || self.trim_file_cache
    }

    /// 把上一次清理结果写入历史字段。
    pub fn record_clean(&mut self, freed_bytes: i64) {
        self.last_freed_bytes = freed_bytes.max(0) as u64;
        self.last_clean_unix = unix_now();
        self.clean_count = self.clean_count.saturating_add(1);
    }
}

/// 配置文件所在目录：%LOCALAPPDATA%\MemCleaner
///
/// 可用环境变量 `MEMCLEANER_DATA_DIR` 覆盖（便携模式 / 自动化测试用）。
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MEMCLEANER_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    base.join("MemCleaner")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

/// 读取配置；文件不存在或内容损坏时返回默认配置（绝不让配置问题阻塞启动）。
pub fn load() -> Config {
    let mut cfg = match fs::read_to_string(config_path()) {
        Ok(text) => serde_json::from_str::<Config>(&text).unwrap_or_default(),
        Err(_) => Config::default(),
    };
    cfg.sanitize();
    cfg
}

/// 原子保存配置：先写临时文件再 rename，避免断电/崩溃留下半个文件。
pub fn save(cfg: &Config) -> std::io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = config_path();
    let tmp = dir.join("config.json.tmp");
    let mut safe = cfg.clone();
    safe.sanitize();
    let text = serde_json::to_string_pretty(&safe).unwrap_or_else(|_| "{}".to_string());
    fs::write(&tmp, text.as_bytes())?;
    fs::rename(&tmp, &path)
}

/// 当前 Unix 时间戳（秒）。
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_sane() {
        let cfg = Config::default();
        assert!(cfg.has_any_item());
        assert!(!cfg.auto_clean_enabled);
        assert_eq!(cfg.clean_working_sets, true);
        assert_eq!(cfg.auto_threshold_percent, 85);
    }

    #[test]
    fn sanitize_fixes_out_of_range() {
        let mut cfg = Config::default();
        cfg.auto_threshold_percent = 250;
        cfg.auto_interval_minutes = 99999;
        cfg.auto_cooldown_seconds = 0;
        cfg.last_freed_bytes = u64::MAX;
        cfg.sanitize();
        assert_eq!(cfg.auto_threshold_percent, 85);
        assert_eq!(cfg.auto_interval_minutes, 0);
        assert_eq!(cfg.auto_cooldown_seconds, 60);
        assert_eq!(cfg.last_freed_bytes, 0);

        let mut ok = Config::default();
        ok.auto_threshold_percent = 0;
        ok.auto_interval_minutes = 30;
        ok.auto_cooldown_seconds = 120;
        ok.sanitize();
        assert_eq!(ok.auto_threshold_percent, 0);
        assert_eq!(ok.auto_interval_minutes, 30);
        assert_eq!(ok.auto_cooldown_seconds, 120);
    }

    #[test]
    fn record_clean_updates_history() {
        let mut cfg = Config::default();
        cfg.record_clean(-5);
        assert_eq!(cfg.last_freed_bytes, 0);
        assert_eq!(cfg.clean_count, 1);
        assert!(cfg.last_clean_unix > 0);
    }
}
