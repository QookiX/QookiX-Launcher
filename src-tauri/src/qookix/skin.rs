//! QookiX 账号自带的皮肤托管。
//!
//! 服务端只接受 PNG（64×64 皮肤 / 64×32 披风），文件按内容哈希落盘由 nginx 直发，
//! 这里只做透传——图片不做任何本地处理，避免给任意图片跑解码器。

use serde_json::{json, Map, Value};
use tauri::State;

use super::api::{self, ApiError};
use crate::state::AppState;

/// 服务端只回文件名，这里补成绝对 URL，前端拿到就能直接当图片地址用
fn absolutize(mut v: Value) -> Value {
    for key in ["skin", "cape"] {
        if let Some(name) = v.get(key).and_then(|x| x.as_str()).map(str::to_string) {
            let url = if name.is_empty() {
                String::new()
            } else {
                format!("{}/skins/{}", api::BASE, name)
            };
            v[key] = json!(url);
        }
    }
    v
}

/// 取自己的皮肤
#[tauri::command]
pub async fn qookix_my_skin(state: State<'_, AppState>) -> Result<Value, ApiError> {
    let v = api::get(&state, "/api/v1/me/skin", Vec::new()).await?;
    Ok(absolutize(v))
}

/// 上传 / 更新皮肤（参数都是可选的，只传 skin 就只换皮肤）
#[tauri::command]
pub async fn qookix_upload_skin(
    state: State<'_, AppState>,
    skin: Option<String>,
    cape: Option<String>,
    model: Option<String>,
) -> Result<Value, ApiError> {
    let mut body = Map::new();
    if let Some(s) = skin.filter(|s| !s.trim().is_empty()) {
        body.insert("skin".into(), json!(s));
    }
    if let Some(c) = cape.filter(|s| !s.trim().is_empty()) {
        body.insert("cape".into(), json!(c));
    }
    if let Some(m) = model.filter(|s| !s.trim().is_empty()) {
        body.insert("model".into(), json!(m));
    }
    if body.is_empty() {
        return Err(ApiError::new(400, "invalid_skin", "没有指定要上传的内容"));
    }
    let v = api::post(&state, "/api/v1/me/skin", Value::Object(body)).await?;
    Ok(absolutize(v))
}

/// 清除自己的皮肤
#[tauri::command]
pub async fn qookix_clear_skin(state: State<'_, AppState>) -> Result<Value, ApiError> {
    api::delete(&state, "/api/v1/me/skin", Vec::new()).await
}

/// 批量取别人的皮肤（好友列表 / 房间成员渲染用）
#[tauri::command]
pub async fn qookix_skins(
    state: State<'_, AppState>,
    uids: Vec<i64>,
) -> Result<Value, ApiError> {
    if uids.is_empty() {
        return Ok(json!({ "skins": {} }));
    }
    let list = uids
        .iter()
        .take(50)
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let v = api::get(
        &state,
        "/api/v1/skins",
        vec![("uids".to_string(), list)],
    )
    .await?;
    // skins 是 { uid: {skin,cape,model} }，逐个补绝对地址
    let mut out = serde_json::Map::new();
    if let Some(map) = v.get("skins").and_then(|m| m.as_object()) {
        for (uid, rec) in map {
            out.insert(uid.clone(), absolutize(rec.clone()));
        }
    }
    Ok(json!({ "skins": out }))
}
