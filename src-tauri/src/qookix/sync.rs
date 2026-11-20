//! 启动器配置云同步（Phase 3）。
//!
//! 只同步「换台电脑也成立」的用户偏好：内存/下载/外观/行为。
//! **不同步**：本机路径（`data_dir` / `java_path` / `background_image`）、
//! 任何密钥（`*_api_key` / `*_client_secret`）、本机 UI 状态（`onboarding_completed` 等）。
//!
//! 为什么白名单而不是黑名单：`Settings` 里以后还会加字段，黑名单迟早漏一个把密钥传上去；
//! 白名单最多是「新字段忘了同步」，不会泄露。

use serde_json::{json, Map, Value};
use tauri::State;

use super::api::{self, ApiError};
use crate::state::AppState;

/// 参与同步的字段。新增字段时想让它跟着走，往这里加一行即可。
const SYNC_KEYS: &[&str] = &[
    // 内存
    "max_memory_mb",
    "min_memory_mb",
    "memory_mode",
    // 下载 / 网络
    "download_threads",
    "download_chunk_threads",
    "mirror",
    "content_mirror",
    // 外观
    "theme",
    "theme_color",
    "background_mode",
    "background_scene",
    "background_quality",
    "background_blur",
    "background_dim",
    "glass_blur",
    "show_news",
    "hidden_nav_items",
    // 行为
    "language",
    "close_behavior",
    "auto_launch",
    "keep_open",
    "auto_update",
    "update_source",
    "translate_provider",
    "body_translate_auto",
];

/// 服务端 KV 里存这一份设置用的 key
const SETTING_KEY: &str = "launcher";

/// 只保留白名单字段（`value` 可能是服务端返回的任何东西，不能直接往本机设置里灌）
fn whitelist(value: &Value) -> Map<String, Value> {
    let Some(settings) = value.as_object() else { return Map::new() };
    let mut portable = Map::new();
    for &key in SYNC_KEYS {
        let Some(value) = settings.get(key) else { continue };
        let allowed = match key {
            "mirror" => preset_id(value, &crate::mirror::presets()),
            "content_mirror" => preset_id(value, &crate::mirror::content_presets()),
            "translate_provider" => matches!(value.as_str(), Some("default" | "baidu_web")),
            _ => true,
        };
        if allowed {
            portable.insert(key.to_owned(), value.clone());
        }
    }
    portable
}

fn preset_id(value: &Value, presets: &Value) -> bool {
    let Some(id) = value.as_str().filter(|id| *id != "custom") else { return false };
    presets.as_array().is_some_and(|items| items.iter().any(|item| {
        item.get("id").and_then(Value::as_str) == Some(id)
    }))
}

/// 当前设置的快照（只含白名单字段）
fn snapshot(state: &AppState) -> Value {
    let all = serde_json::to_value(state.settings().clone()).unwrap_or(Value::Null);
    Value::Object(whitelist(&all))
}

/// 上传本机配置到云端（覆盖）
#[tauri::command]
pub async fn qookix_sync_upload(state: State<'_, AppState>) -> Result<Value, ApiError> {
    let value = snapshot(&state);
    let keys = value.as_object().map(|o| o.len()).unwrap_or(0);
    let data = api::put(
        &state,
        "/api/v1/me/settings",
        json!({ "key": SETTING_KEY, "value": value }),
    )
    .await?;
    Ok(json!({
        "keys": keys,
        "updated_at": data.get("updated_at").cloned().unwrap_or(Value::Null),
    }))
}

/// 从云端拉配置并应用到本机（覆盖）
#[tauri::command]
pub async fn qookix_sync_download(state: State<'_, AppState>) -> Result<Value, ApiError> {
    let data = api::get(
        &state,
        "/api/v1/me/settings",
        vec![("key".to_string(), SETTING_KEY.to_string())],
    )
    .await?;
    let item = data
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned();
    let Some(item) = item else {
        return Ok(json!({ "applied": 0, "empty": true }));
    };
    let value = item.get("value").cloned().unwrap_or(Value::Null);
    let patch = Value::Object(whitelist(&value));
    let applied = patch.as_object().map(|o| o.len()).unwrap_or(0);
    crate::settings::update_settings(&state, patch).map_err(ApiError::internal)?;
    Ok(json!({
        "applied": applied,
        "empty": false,
        "updated_at": item.get("updated_at").cloned().unwrap_or(Value::Null),
    }))
}

// ---- 通用 KV：给收藏夹这类前端持有的数据用 ----

#[tauri::command]
pub async fn qookix_kv_get(
    state: State<'_, AppState>,
    key: String,
) -> Result<Value, ApiError> {
    let data = api::get(
        &state,
        "/api/v1/me/settings",
        vec![("key".to_string(), key)],
    )
    .await?;
    let item = data
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned();
    Ok(match item {
        Some(it) => json!({
            "value": it.get("value").cloned().unwrap_or(Value::Null),
            "updated_at": it.get("updated_at").cloned().unwrap_or(Value::Null),
        }),
        None => json!({ "value": Value::Null, "updated_at": Value::Null }),
    })
}

#[tauri::command]
pub async fn qookix_kv_put(
    state: State<'_, AppState>,
    key: String,
    value: Value,
) -> Result<Value, ApiError> {
    api::put(
        &state,
        "/api/v1/me/settings",
        json!({ "key": key, "value": value }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 白名单必须挡住密钥与本机路径——这条挂了就是安全事故
    #[test]
    fn whitelist_drops_secrets_and_local_paths() {
        let raw = json!({
            "theme": "dark",
            "max_memory_mb": 8192,
            "curseforge_api_key": "secret",
            "translate_api_key": "secret",
            "shimu_client_secret": "secret",
            "data_dir": "C:/Users/me/AppData",
            "java_path": "C:/Java/bin/java.exe",
            "background_image": "C:/pic.png",
            "onboarding_completed": true,
            "share_activity": false,
        });
        let kept = whitelist(&raw);
        assert_eq!(kept.get("theme").and_then(|v| v.as_str()), Some("dark"));
        assert_eq!(kept.get("max_memory_mb").and_then(|v| v.as_u64()), Some(8192));
        for blocked in [
            "curseforge_api_key",
            "translate_api_key",
            "shimu_client_secret",
            "data_dir",
            "java_path",
            "background_image",
            "onboarding_completed",
        ] {
            assert!(!kept.contains_key(blocked), "{blocked} 不应进入同步白名单");
        }
    }

    #[test]
    fn sync_keeps_portable_settings_and_omits_local_endpoints_and_arguments() {
        let fields = json!({
            "theme": "dark", "download_threads": 6, "mirror": "official",
            "content_mirror": "custom", "content_mirror_custom": "https://user:password@host/",
            "translate_provider": "custom", "translate_api_base": "https://private/",
            "translate_api_model": "local", "proxy_mode": "custom",
            "proxy": "http://user:password@proxy", "jvm_args": "-Dtoken=secret",
            "game_args": "--path /home/alice", "data_dir": "/home/alice"
        });
        let synced = Value::Object(whitelist(&fields));
        assert_eq!(synced, json!({ "download_threads": 6, "mirror": "official", "theme": "dark" }));
    }
}
