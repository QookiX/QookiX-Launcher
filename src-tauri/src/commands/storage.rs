use crate::state::AppState;
use tauri::{AppHandle, Manager};

// Storage
// ---------------------------------------------------------------------------

/// 获取存储统计：优先返回上次扫描的缓存，无缓存时实时扫描
#[tauri::command]
pub async fn get_storage_stats(app: AppHandle) -> Result<crate::storage::StorageStats, String> {
    run_storage_task(app, crate::storage::get_storage_stats).await
}

/// 强制重新扫描存储并返回最新统计
#[tauri::command]
pub async fn refresh_storage_stats(app: AppHandle) -> Result<crate::storage::StorageStats, String> {
    run_storage_task(app, crate::storage::refresh_storage_stats).await
}

/// 清除可安全清理的缓存，返回释放的空间
#[tauri::command]
pub async fn clear_cache(app: AppHandle) -> Result<crate::storage::CacheClearResult, String> {
    run_storage_task(app, crate::storage::clear_cache).await
}

async fn run_storage_task<T: Send + 'static>(
    app: AppHandle,
    run: fn(&AppState) -> T,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || run(&app.state::<AppState>()))
        .await
        .map_err(|err| err.to_string())
}
