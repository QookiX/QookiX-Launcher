use tauri::Manager;

pub mod accounts;
pub mod browse;
pub mod crash;
pub mod files;
pub mod home;
pub mod hosted;
pub mod instances;
pub mod lan_sync;
pub mod multiplayer;
pub mod nbt;
pub mod options;
pub mod pins;
pub mod resourcepack;
pub mod schematic;
pub mod schematic_resources;
pub mod settings;
pub mod skins;
pub mod storage;
pub mod toolbox;
pub mod version;
pub mod world_backup;

pub use accounts::*;
pub use browse::*;
pub use crash::*;
pub use files::*;
pub use home::*;
pub use hosted::*;
pub use instances::*;
pub use multiplayer::*;
pub use pins::*;
pub use resourcepack::*;
pub use schematic::*;
pub use schematic_resources::*;
pub use settings::*;
pub use skins::*;
pub use storage::*;
pub use toolbox::*;
pub use version::*;
pub use world_backup::*;
pub use nbt::*;
pub use options::*;

/// 开发者工具：程序化打开主窗口的浏览器 DevTools。
///
/// `tauri.conf.json` 的 `devtools: false` 已关掉 F12 / 右键「检查」等用户入口
/// （WebView2 的 `SetAreDevToolsEnabled(false)`），但 `OpenDevToolsWindow()`
/// 不受该设置影响，所以这里仍能在前端开关放行时打开。
/// 命令本身不做权限判断：浏览器原生快捷键已被禁用，调用与否由前端开关决定。
#[tauri::command]
pub fn open_devtools(app: tauri::AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;
    win.open_devtools();
    Ok(())
}
