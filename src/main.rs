//! MemCleaner —— 极简、低占用的 Windows 内存清理工具。
//!
//! 清理逻辑逆向自 PCL 启动器 2.13.1.1 的「内存优化」：
//! 启用 SeProfileSingleProcessPrivilege 后依次执行
//! `NtSetSystemInformation(80, {2,3,4}, 4)`（清空工作集 → 刷新已修改页 → 清除待机列表）。
//!
//! 命令行参数：
//! - 无参数：启动主界面
//! - `--autostart`：静默启动到托盘（计划任务使用）
//! - `--elevated`：标记以管理员身份重启（内部使用）
//! - `--apply-autostart`：启动后应用待生效的开机自启设置（内部使用）
//! - `--version` / `--help`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod platform;
mod selftest;
mod sys;
mod ui;

use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("MemCleaner {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return;
    }
    // 自检模式：不开窗口、不加载 UI，用于验证清理效果（可被计划任务/UAC 提升后调用）。
    if args.iter().any(|a| a == "--self-test") {
        attach_parent_console();
        std::process::exit(selftest::run(&args));
    }
    // 无界面维护模式：安装 / 卸载开机自启计划任务（需要管理员权限）。
    if args.iter().any(|a| a == "--install-autostart" || a == "--uninstall-autostart") {
        attach_parent_console();
        let install = args.iter().any(|a| a == "--install-autostart");
        let exe = platform::elevate::current_exe();
        let result = if install {
            platform::autostart::install(&exe)
        } else {
            platform::autostart::uninstall()
        };
        match result {
            Ok(()) => {
                println!("{}", if install { "autostart installed" } else { "autostart removed" });
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("autostart failed: {e}");
                std::process::exit(5);
            }
        }
    }

    // 必须在任何窗口/DC 创建之前设置 DPI 感知。
    platform::window::init_dpi_awareness();

    // 单实例：提权重启时旧实例正在退出，需要短暂重试等待互斥体释放。
    let wait_for_previous = args
        .iter()
        .any(|a| a == "--elevated" || a == "--apply-autostart" || a == "--clean-on-start");
    let _guard = if wait_for_previous {
        acquire_with_retry(4000)
    } else {
        platform::single_instance::acquire()
    };
    let Some(_guard) = _guard else {
        // 已有实例：唤醒它的窗口后退出。
        platform::single_instance::wake_existing_instance();
        return;
    };

    let cfg = config::load();
    let hidden = app::start_hidden(&args);
    let apply_autostart = args.iter().any(|a| a == "--apply-autostart");
    let clean_on_start = args.iter().any(|a| a == "--clean-on-start");

    if !platform::window::register_class(Some(app::wndproc)) {
        app::log_line("register window class failed");
        return;
    }
    let dpi = platform::window::system_dpi();
    let Some(hwnd) = platform::window::create_main_window(Some(app::wndproc), dpi) else {
        app::log_line("create window failed");
        return;
    };

    let boxed = Box::new(app::App::new(hwnd, cfg, !hidden, apply_autostart, clean_on_start));
    let app_ptr = Box::into_raw(boxed);
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize);
    }
    // 高权限实例放行低权限实例的唤醒消息（UIPI）。
    platform::window::allow_message_from_lower_integrity(hwnd, platform::WM_APP_SHOW);

    run_message_loop();

    // 退出：释放 App（托盘图标、GDI 资源、渲染器缓冲）。
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut app::App;
        if !ptr.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(ptr));
        }
    }
}

fn run_message_loop() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret <= 0 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn acquire_with_retry(timeout_ms: u64) -> Option<platform::single_instance::InstanceGuard> {
    let start = Instant::now();
    loop {
        if let Some(guard) = platform::single_instance::acquire() {
            return Some(guard);
        }
        if start.elapsed() >= Duration::from_millis(timeout_ms) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn print_help() {
    println!("MemCleaner {} —— 简洁的 Windows 内存清理工具", env!("CARGO_PKG_VERSION"));
    println!();
    println!("用法：MemCleaner [选项]");
    println!("  --autostart        静默启动到托盘（开机自启计划任务使用）");
    println!("  --elevated         以管理员身份重启（内部使用）");
    println!("  --apply-autostart  启动后应用开机自启设置（内部使用）");
    println!("  --self-test [--out 文件]  命令行自检（验证清理效果，写 JSON 结果）");
    println!("  --install-autostart       安装开机自启计划任务（需管理员）");
    println!("  --uninstall-autostart     卸载开机自启计划任务（需管理员）");
    println!("  --version          显示版本");
    println!("  --help             显示帮助");
    println!();
    println!("清理原理与 PCL 启动器一致：");
    println!("  1. 启用 SeProfileSingleProcessPrivilege 权限");
    println!("  2. NtSetSystemInformation(80, 2)  清空所有进程工作集");
    println!("  3. NtSetSystemInformation(80, 3)  刷新已修改页面列表");
    println!("  4. NtSetSystemInformation(80, 4)  清除待机列表");
}

/// 防止未使用导入警告（main 里只用到部分消息类型）。
#[allow(dead_code)]
fn _keep_types(_: HWND, _: WPARAM, _: LPARAM) {}

/// 附加到父进程控制台，让 `--self-test` 的 println! 输出可见（GUI 子系统默认无控制台）。
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
