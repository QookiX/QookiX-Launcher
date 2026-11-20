//! 自定义主页布局文件（`home.json`）的读写。
//!
//! 前端只负责渲染，文件的落地位置由后端决定：数据目录根下的 `home.json`。
//! 用户可以用任意文本编辑器改它，改完回首页点「重新加载」即可，不需要重启启动器。
//! 文件缺失时前端会写入一份带注释的默认模板；「恢复默认」= 删掉这个文件。

use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

#[derive(Serialize)]
pub struct HomeLayoutInfo {
    /// 布局文件的绝对路径（前端展示 / 打开用）
    pub path: String,
    /// 文件是否已存在
    pub exists: bool,
    /// 文件内容；不存在时为空串
    pub content: String,
}

fn layout_path(state: &AppState) -> PathBuf {
    state.root.join("home.json")
}

/// 读取布局文件。文件不存在不算错误——前端会拿到 exists=false 后写默认模板。
#[tauri::command]
pub fn home_layout_read(state: State<AppState>) -> Result<HomeLayoutInfo, String> {
    let path = layout_path(&state);
    let path_str = path.to_string_lossy().to_string();
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(HomeLayoutInfo {
            path: path_str,
            exists: true,
            content,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HomeLayoutInfo {
            path: path_str,
            exists: false,
            content: String::new(),
        }),
        Err(e) => Err(format!("读取主页布局失败: {e}")),
    }
}

/// 覆盖写入布局文件（先写临时文件再替换，避免用户正在编辑器里保存时读到半截内容）。
#[tauri::command]
pub fn home_layout_save(state: State<AppState>, content: String) -> Result<(), String> {
    let path = layout_path(&state);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    crate::fsutil::atomic_write(&path, content.as_bytes())
        .map_err(|e| format!("保存主页布局失败: {e}"))
}

/// 恢复默认：删除布局文件，下次读取时前端会重新生成模板。
#[tauri::command]
pub fn home_layout_reset(state: State<AppState>) -> Result<(), String> {
    match std::fs::remove_file(layout_path(&state)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除主页布局失败: {e}")),
    }
}
