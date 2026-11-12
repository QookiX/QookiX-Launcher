//! 陶瓦联机房间目录：上报、心跳、大厅、邀请、举报、通知。

use super::api::{self, ApiError};
use super::poller::start_share_task;
use super::{status, RoomShare};
use crate::state::AppState;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 校验房间码：服务端要求 4-64 位可见 ASCII 且不含空格
fn normalize_code(code: &str) -> Result<String, ApiError> {
    let code = code.trim().to_string();
    let n = code.chars().count();
    let ok = (4..=64).contains(&n) && code.chars().all(|c| c.is_ascii_graphic());
    if !ok {
        return Err(ApiError::new(
            400,
            "invalid_room_code",
            "房间码无效（应为 4-64 位可见字符）",
        ));
    }
    Ok(code)
}

fn clamp_limit(limit: Option<u32>) -> String {
    limit.unwrap_or(20).clamp(1, 100).to_string()
}

/// 取下当前分享并置取消标志，返回旧的 room_id
fn take_share(state: &AppState) -> Option<String> {
    let share = { state.qookix().share.take() };
    share.map(|s| {
        s.cancel.store(true, Ordering::SeqCst);
        s.room_id
    })
}

/// 把房间分享到目录，并启动绑定陶瓦会话的心跳任务
#[tauri::command]
pub async fn qookix_share_room(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    code: String,
    visibility: Option<String>,
    title: Option<String>,
    game_version: Option<String>,
    modpack: Option<String>,
    play_type: Option<String>,
    // pack: 环境清单 JSON（房主勾选「共享环境」时由客户端生成）
    pack: Option<String>,
    // mods_count: 清单里的 mod 数量，仅用于列表展示
    mods_count: Option<u32>,
    // pack_net: P2P 连接信息（房主开了文件服务才有）
    pack_net: Option<String>,
) -> Result<Value, ApiError> {
    let code = normalize_code(&code)?;
    let pack = pack.unwrap_or_default();
    let mods_count = mods_count.unwrap_or(0).min(2000);
    let visibility = match visibility.as_deref() {
        Some("public") => "public",
        _ => "friends",
    }
    .to_string();
    let title: String = title
        .unwrap_or_default()
        .trim()
        .chars()
        .take(60)
        .collect();

    // 换房或重复点按时，先把旧条目关掉
    if let Some(old) = take_share(&state) {
        let _ = api::post(&state, "/api/v1/rooms/close", json!({ "room_id": old })).await;
    }

    let mut body = json!({ "code": code.clone(), "visibility": visibility.clone() });
    if !title.is_empty() {
        body["title"] = json!(title.clone());
    }
    if let Some(v) = game_version.filter(|v| !v.trim().is_empty()) {
        body["game_version"] = json!(v.trim());
    }
    if let Some(v) = modpack.filter(|v| !v.trim().is_empty()) {
        body["modpack"] = json!(v.trim());
    }
    // 玩法类型是服务端固定枚举，空串表示不标注
    if let Some(v) = play_type.filter(|v| !v.trim().is_empty()) {
        body["play_type"] = json!(v.trim().to_ascii_lowercase());
    }
    // 环境清单：服务端只校验体积并算指纹（pack_hash / pack_size 由服务端生成）
    if !pack.trim().is_empty() {
        let v = pack.trim();
        if v.len() > 65536 {
            return Err(ApiError::new(
                400,
                "invalid_room",
                "环境清单过大（超过 64KB），请减少模组数量或只共享部分清单",
            ));
        }
        body["pack_json"] = json!(v);
        body["mods_count"] = json!(mods_count);
        if let Some(n) = pack_net.filter(|n| !n.trim().is_empty()) {
            // 服务端只做长度约束（≤255），不解析内容
            body["pack_net"] = json!(n.trim());
        }
    }

    let data = api::post(&state, "/api/v1/rooms", body).await?;
    let room_id = data
        .get("room")
        .and_then(|r| r.get("id"))
        .and_then(|i| i.as_str())
        .ok_or_else(|| ApiError::internal("上报房间后服务端未返回 room.id"))?
        .to_string();

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut s = state.qookix();
        s.share = Some(RoomShare {
            room_id: room_id.clone(),
            code: code.clone(),
            title: title.clone(),
            visibility: visibility.clone(),
            pack: pack.clone(),
            mods_count,
            cancel: cancel.clone(),
        });
    }
    start_share_task(
        app,
        cancel,
        room_id,
        code,
        title,
        visibility,
        pack,
        mods_count,
    );

    Ok(json!({
        "room": data.get("room").cloned().unwrap_or(Value::Null),
        "created": data.get("created").cloned().unwrap_or(Value::Bool(false)),
        "share": status(&state).get("share").cloned().unwrap_or(Value::Null),
    }))
}

/// 停止分享（关闭目录条目，不影响陶瓦本身的联机）
#[tauri::command]
pub async fn qookix_stop_share(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    if let Some(room_id) = take_share(&state) {
        // 关不掉也无所谓：TTL 到了服务端会自动过期
        let _ = api::post(&state, "/api/v1/rooms/close", json!({ "room_id": room_id })).await;
    }
    Ok(status(&state))
}

/// 房间列表（命令与后台轮询共用）
pub(crate) async fn fetch_rooms(
    state: &AppState,
    scope: &str,
    page: u32,
    limit: u32,
) -> Result<Value, ApiError> {
    api::get(
        state,
        "/api/v1/rooms",
        vec![
            ("scope".to_string(), scope.to_string()),
            ("page".to_string(), page.max(1).to_string()),
            ("limit".to_string(), limit.clamp(1, 100).to_string()),
        ],
    )
    .await
}

/// 好友房间列表（scope = friends 或 mine）
#[tauri::command]
pub async fn qookix_rooms(
    state: tauri::State<'_, AppState>,
    scope: Option<String>,
    page: Option<u32>,
    limit: Option<u32>,
) -> Result<Value, ApiError> {
    let scope = match scope.as_deref() {
        Some("mine") => "mine",
        _ => "friends",
    };
    fetch_rooms(
        &state,
        scope,
        page.unwrap_or(1),
        limit.unwrap_or(20),
    )
    .await
}

/// 公开大厅
#[tauri::command]
pub async fn qookix_public_rooms(
    state: tauri::State<'_, AppState>,
    q: Option<String>,
    page: Option<u32>,
    limit: Option<u32>,
    version: Option<String>,
    play_type: Option<String>,
    min_players: Option<u32>,
    max_players: Option<u32>,
    sort: Option<String>,
) -> Result<Value, ApiError> {
    let mut query = vec![
        ("page".to_string(), page.unwrap_or(1).max(1).to_string()),
        ("limit".to_string(), clamp_limit(limit)),
    ];
    if let Some(q) = q.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        query.push(("q".to_string(), q));
    }
    if let Some(v) = version.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        query.push(("version".to_string(), v));
    }
    if let Some(v) = play_type.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        query.push(("play_type".to_string(), v.to_ascii_lowercase()));
    }
    if let Some(n) = min_players.filter(|n| *n > 0) {
        query.push(("min_players".to_string(), n.to_string()));
    }
    if let Some(n) = max_players.filter(|n| *n > 0) {
        query.push(("max_players".to_string(), n.to_string()));
    }
    if let Some(s) = sort.filter(|s| s == "players") {
        query.push(("sort".to_string(), s));
    }
    api::get(&state, "/api/v1/rooms/public", query).await
}

#[tauri::command]
pub async fn qookix_room_detail(
    state: tauri::State<'_, AppState>,
    room_id: String,
) -> Result<Value, ApiError> {
    api::get(
        &state,
        "/api/v1/rooms/detail",
        vec![("room_id".to_string(), room_id)],
    )
    .await
}

#[tauri::command]
pub async fn qookix_invite(
    state: tauri::State<'_, AppState>,
    room_id: String,
    to_uid: i64,
) -> Result<Value, ApiError> {
    api::post(
        &state,
        "/api/v1/rooms/invites",
        json!({ "room_id": room_id, "to_uid": to_uid }),
    )
    .await
}

#[tauri::command]
pub async fn qookix_invites(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    api::get(&state, "/api/v1/rooms/invites", Vec::new()).await
}

/// 接受邀请：返回的 room 里带 `code`，前端直接拿去加入陶瓦
#[tauri::command]
pub async fn qookix_accept_invite(
    state: tauri::State<'_, AppState>,
    invite_id: i64,
) -> Result<Value, ApiError> {
    api::post(
        &state,
        "/api/v1/rooms/invites/accept",
        json!({ "invite_id": invite_id }),
    )
    .await
}

#[tauri::command]
pub async fn qookix_decline_invite(
    state: tauri::State<'_, AppState>,
    invite_id: i64,
) -> Result<Value, ApiError> {
    api::post(
        &state,
        "/api/v1/rooms/invites/decline",
        json!({ "invite_id": invite_id }),
    )
    .await
}

#[tauri::command]
pub async fn qookix_report_room(
    state: tauri::State<'_, AppState>,
    room_id: String,
    reason: String,
) -> Result<Value, ApiError> {
    let reason = reason.trim().to_string();
    let n = reason.chars().count();
    if !(3..=300).contains(&n) {
        return Err(ApiError::new(400, "invalid_reason", "举报理由需为 3-300 个字符"));
    }
    api::post(
        &state,
        "/api/v1/rooms/report",
        json!({ "room_id": room_id, "reason": reason }),
    )
    .await
}

/// 拉一次通知增量。`server_time` 是服务端请求开始时刻，而过滤是严格的
/// `created_at > since`，同一秒内后到的事件会被永久漏掉，所以游标回退 2 秒，
/// 由前端按 id 去重。
pub(crate) async fn fetch_notifications(state: &AppState) -> Result<Value, ApiError> {
    let since = {
        let s = state.qookix();
        s.cursor.saturating_sub(2)
    };
    let data = api::get(
        state,
        "/api/v1/notifications",
        vec![("since".to_string(), since.to_string())],
    )
    .await?;
    if let Some(t) = data.get("server_time").and_then(|v| v.as_u64()) {
        let mut s = state.qookix();
        if t > s.cursor {
            s.cursor = t;
        }
    }
    Ok(data)
}

#[tauri::command]
pub async fn qookix_notifications(state: tauri::State<'_, AppState>) -> Result<Value, ApiError> {
    fetch_notifications(&state).await
}
