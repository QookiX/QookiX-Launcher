use crate::state::AppState;
use tauri::State;

// Pinned items (首页快捷卡片 / 侧边栏图标)
// ---------------------------------------------------------------------------

/// 读取全部固定项（首页与侧边栏各自独立存储）。
#[tauri::command]
pub fn get_pins(state: State<AppState>) -> Result<Vec<crate::models::PinItem>, String> {
    crate::pins::load_pins(&state)
}

/// 在数据库事务内替换全部固定项。
#[tauri::command]
pub fn set_pins(state: State<AppState>, items: Vec<crate::models::PinItem>) -> Result<(), String> {
    crate::pins::save_pins(&state, &items)
}
