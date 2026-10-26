//! WebView2 现代安装器入口。
//!
//! 本 crate 的实现参考自 Axolotl Launcher
//! (https://github.com/Axolotl-Launcher/Axolotl, GPL-3.0) 的 installer-ui 模块。
//! 完整版权声明见 THIRD_PARTY_NOTICES.md。
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod windows;

#[cfg(windows)]
fn main() {
    if let Err(error) = windows::run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("QookiXInstallerUI is only available on Windows");
}
