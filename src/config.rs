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

/// 强调色配色方案。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AccentScheme {
    /// 极光蓝（默认）。
    Blue,
    /// 薄荷绿。
    Mint,
    /// 紫罗兰。
    Violet,
    /// 琥珀橙。
    Amber,
    /// 樱粉。
    Rose,
    /// 石墨灰。
    Graphite,
}

impl Default for AccentScheme {
    fn default() -> Self {
        AccentScheme::Blue
    }
}

impl AccentScheme {
    /// 设置页展示名。
    pub fn display_name(self) -> &'static str {
        match self {
            AccentScheme::Blue => "极光蓝",
            AccentScheme::Mint => "薄荷绿",
            AccentScheme::Violet => "紫罗兰",
            AccentScheme::Amber => "琥珀橙",
            AccentScheme::Rose => "樱粉",
            AccentScheme::Graphite => "石墨灰",
        }
    }

    /// 设置页循环切换的下一个方案。
    pub fn next(self) -> Self {
        match self {
            AccentScheme::Blue => AccentScheme::Mint,
            AccentScheme::Mint => AccentScheme::Violet,
            AccentScheme::Violet => AccentScheme::Amber,
            AccentScheme::Amber => AccentScheme::Rose,
            AccentScheme::Rose => AccentScheme::Graphite,
            AccentScheme::Graphite => AccentScheme::Blue,
        }
    }

    pub const ALL: [AccentScheme; 6] = [
        AccentScheme::Blue,
        AccentScheme::Mint,
        AccentScheme::Violet,
        AccentScheme::Amber,
        AccentScheme::Rose,
        AccentScheme::Graphite,
    ];
}

/// 背景皮肤（预设）或自定义图片。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkinPreset {
    /// 默认（仿 Mica 渐变 + 光晕）。
    Default,
    /// 极光。
    Aurora,
    /// 星云。
    Nebula,
    /// 晨雾。
    Dawn,
    /// 暮色。
    Dusk,
    /// 自定义图片（`skin_image` 非空时生效，否则回退默认）。
    Custom,
}

impl Default for SkinPreset {
    fn default() -> Self {
        SkinPreset::Default
    }
}

impl SkinPreset {
    pub fn display_name(self) -> &'static str {
        match self {
            SkinPreset::Default => "默认",
            SkinPreset::Aurora => "极光",
            SkinPreset::Nebula => "星云",
            SkinPreset::Dawn => "晨雾",
            SkinPreset::Dusk => "暮色",
            SkinPreset::Custom => "自定义图片",
        }
    }

    /// 在预设之间循环（不进入 Custom；Custom 由选择图片/清除图片控制）。
    pub fn next(self) -> Self {
        match self {
            SkinPreset::Default => SkinPreset::Aurora,
            SkinPreset::Aurora => SkinPreset::Nebula,
            SkinPreset::Nebula => SkinPreset::Dawn,
            SkinPreset::Dawn => SkinPreset::Dusk,
            SkinPreset::Dusk => SkinPreset::Default,
            SkinPreset::Custom => SkinPreset::Default,
        }
    }
}

/// 应用配置（全部字段带默认值，旧版本配置缺字段也能读）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    // ---------- 自动清理 ----------
    /// 是否启用自动清理（需要管理员权限）。
    pub auto_clean_enabled: bool,
    /// 内存使用率高于该百分比时自动清理（30–100）。
    pub auto_threshold_percent: u8,
    /// 固定间隔自动清理（分钟）；0 表示关闭。
    pub auto_interval_minutes: u32,
    /// 两次自动清理之间的最小冷却时间（秒）。
    ///
    /// v1.3.0 起下限为 180 秒：此前的 60 秒冷却会让「使用率长期高于阈值」的机器
    /// 每分钟清理一次，而每次清理（尤其 FlushModifiedList/收缩文件缓存）都要占用
    /// 数秒磁盘 IO，反而造成卡顿。自动清理的目标是「缓解压力」，不是「压住数字」。
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
    /// 强调色配色方案。
    pub accent: AccentScheme,
    /// 背景皮肤预设；`Custom` 时使用 `skin_image`。
    pub skin_preset: SkinPreset,
    /// 自定义皮肤图片路径（空 = 未选择）。
    pub skin_image: String,
    /// 自定义皮肤图片叠加不透明度（0–100）。
    pub skin_opacity: u8,
    /// 卡片/面板不透明度（0–100）。
    pub panel_opacity: u8,
    /// 整个窗口的不透明度（50–100，分层窗口 alpha）。
    pub window_opacity: u8,
    /// 托盘图标是否直接显示内存占用率。
    pub tray_show_percent: bool,
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
            auto_cooldown_seconds: 300,
            clean_working_sets: true,
            clean_modified: true,
            clean_standby: true,
            clean_low_priority: false,
            trim_file_cache: false,
            theme: ThemeMode::System,
            accent: AccentScheme::Blue,
            skin_preset: SkinPreset::Default,
            skin_image: String::new(),
            skin_opacity: 55,
            panel_opacity: 100,
            window_opacity: 100,
            tray_show_percent: true,
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
    /// 范围校验：把越界/不合理的值钳制或回退默认值。
    /// 在 load() 解析后、save() 写入前都调用一次，保证配置文件永远合法。
    pub fn sanitize(&mut self) {
        // 阈值：30–100（直接输入/滑块都限制在该范围）。
        self.auto_threshold_percent = self.auto_threshold_percent.clamp(30, 100);
        // 固定间隔：0..=1440 分钟（一天），越界回退 0（关闭）。
        if self.auto_interval_minutes > 1440 {
            self.auto_interval_minutes = 0;
        }
        // 冷却时间：180..=3600 秒（v1.3.0 提高下限，避免过于频繁的清理拖慢系统）。
        if !(180..=3600).contains(&self.auto_cooldown_seconds) {
            self.auto_cooldown_seconds = 180;
        }
        // 透明度范围（面板下限 20，避免把界面调到看不见）。
        self.skin_opacity = self.skin_opacity.min(100);
        self.panel_opacity = self.panel_opacity.clamp(20, 100);
        self.window_opacity = self.window_opacity.clamp(50, 100);
        // 自定义皮肤必须有图片路径，否则回退默认皮肤。
        if self.skin_preset == SkinPreset::Custom && self.skin_image.trim().is_empty() {
            self.skin_preset = SkinPreset::Default;
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

    /// 自定义皮肤文件名（仅用于界面展示）。
    pub fn skin_file_name(&self) -> String {
        std::path::Path::new(&self.skin_image)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
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
        // 兼容 Windows 记事本/PowerShell 写入的 UTF-8 BOM，避免解析失败导致配置丢失。
        Ok(text) => serde_json::from_str::<Config>(text.trim_start_matches('\u{feff}'))
            .unwrap_or_default(),
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
        assert_eq!(cfg.accent, AccentScheme::Blue);
        assert_eq!(cfg.skin_preset, SkinPreset::Default);
        assert!(cfg.tray_show_percent);
    }

    #[test]
    fn sanitize_fixes_out_of_range() {
        let mut cfg = Config::default();
        cfg.auto_threshold_percent = 250;
        cfg.auto_interval_minutes = 99999;
        cfg.auto_cooldown_seconds = 0;
        cfg.last_freed_bytes = u64::MAX;
        cfg.skin_opacity = 200;
        cfg.panel_opacity = 200;
        cfg.window_opacity = 5;
        cfg.skin_preset = SkinPreset::Custom;
        cfg.skin_image.clear();
        cfg.sanitize();
        assert_eq!(cfg.auto_threshold_percent, 100);
        assert_eq!(cfg.auto_interval_minutes, 0);
        // v1.3.0：冷却时间下限提高到 180 秒。
        assert_eq!(cfg.auto_cooldown_seconds, 180);
        assert_eq!(cfg.last_freed_bytes, 0);
        assert_eq!(cfg.skin_opacity, 100);
        assert_eq!(cfg.panel_opacity, 100);
        // 下限保护
        let mut low = Config::default();
        low.panel_opacity = 0;
        low.sanitize();
        assert_eq!(low.panel_opacity, 20);
        assert_eq!(cfg.window_opacity, 50);
        assert_eq!(cfg.skin_preset, SkinPreset::Default);

        let mut ok = Config::default();
        ok.auto_threshold_percent = 30;
        ok.auto_interval_minutes = 30;
        ok.auto_cooldown_seconds = 600;
        ok.window_opacity = 100;
        ok.sanitize();
        assert_eq!(ok.auto_threshold_percent, 30);
        assert_eq!(ok.auto_interval_minutes, 30);
        assert_eq!(ok.auto_cooldown_seconds, 600);
        assert_eq!(ok.window_opacity, 100);
    }

    #[test]
    fn accent_and_skin_cycles() {
        let mut a = AccentScheme::Blue;
        for _ in 0..AccentScheme::ALL.len() {
            a = a.next();
        }
        assert_eq!(a, AccentScheme::Blue);
        let mut s = SkinPreset::Default;
        for _ in 0..5 {
            s = s.next();
        }
        assert_eq!(s, SkinPreset::Default);
        assert_eq!(SkinPreset::Custom.next(), SkinPreset::Default);
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
