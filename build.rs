//! 构建脚本：为 Windows 可执行文件嵌入图标。
//!
//! 仅在 Windows 目标下生效，Android/Linux/macOS 构建完全跳过。
//! 手写 .rc 并调用同目录工具链的 windres，避免引入额外 crate 依赖。

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest_dir.join("assets").join("app.ico");
    if !icon.exists() {
        println!("cargo:warning=assets/app.ico 不存在，跳过图标嵌入");
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc_path = out_dir.join("app.rc");
    let res_path = out_dir.join("app.res.o");

    // 资源脚本：主图标 ID 1，供 exe 在资源管理器/任务栏显示。
    let rc = format!(
        "1 ICON \"{}\"\n",
        icon.display().to_string().replace('\\', "\\\\")
    );
    std::fs::write(&rc_path, rc).expect("写入 app.rc 失败");

    // windres 随 Rust 的 GNU 工具链提供；找不到时静默跳过，不影响编译。
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "windres".to_string());
    let status = Command::new(&windres)
        .args(["-i"])
        .arg(&rc_path)
        .arg("-o")
        .arg(&res_path)
        .arg("--output-format=coff")
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
        _ => println!("cargo:warning=windres 不可用，未嵌入 exe 图标"),
    }
}
