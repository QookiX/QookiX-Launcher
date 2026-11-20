use crate::models::PinItem;
use crate::state::AppState;

/// 读取固定项列表，区分没有记录与无法读取或解析。
pub fn load_pins(state: &AppState) -> Result<Vec<PinItem>, String> {
    let conn = state.db();
    crate::db::require_persistent(&conn)?;
    crate::db::load_rows(&conn, "pins")
}

/// 将固定项列表写入数据库。
pub fn save_pins(state: &AppState, items: &[PinItem]) -> Result<(), String> {
    let vals = items.iter().map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut conn = state.db();
    crate::db::require_persistent(&conn)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    crate::db::save_rows(&tx, "pins", &vals)?;
    tx.commit().map_err(|e| e.to_string())
}
