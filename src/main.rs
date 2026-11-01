//! Sports simulation：无参数启动 GUI，带子命令进入 CLI（青龙等定时任务用）。
//! full 构建（默认）含 GUI + CLI；`--no-default-features` 仅 CLI。

use sportssimulation::cli;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        #[cfg(feature = "gui")]
        run_gui();
        #[cfg(not(feature = "gui"))]
        {
            eprintln!("当前为 CLI 构建，子命令用法：sportssimulation help");
            std::process::exit(1);
        }
    } else {
        let code = cli::main(args);
        std::process::exit(code);
    }
}

#[cfg(all(windows, feature = "gui"))]
fn free_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn FreeConsole() -> i32;
    }
    unsafe {
        FreeConsole();
    }
}

#[cfg(not(all(windows, feature = "gui")))]
fn free_console() {}

/// 窗口/任务栏图标。编译期内嵌 PNG，运行时解码为 egui 需要的 RGBA。
#[cfg(feature = "gui")]
fn window_icon() -> Option<eframe::egui::IconData> {
    const PNG: &[u8] = include_bytes!("../assets/window_icon.png");
    let img = image::load_from_memory_with_format(PNG, image::ImageFormat::Png).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(eframe::egui::IconData {
        rgba: rgba.into_raw(),
        width: w,
        height: h,
    })
}

#[cfg(feature = "gui")]
fn run_gui() {
    free_console();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([880.0, 720.0])
        .with_title("Sports simulation");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = sportssimulation::run_gui(options);
    if let Err(e) = result {
        eprintln!("GUI 启动失败: {e}");
        std::process::exit(1);
    }
}
