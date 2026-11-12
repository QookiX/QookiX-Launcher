//! 后台轮询：在线心跳、通知增量、好友列表；以及绑定陶瓦会话的房间心跳。
//!
//! 全部放在 Rust 侧：前端切页面、掉帧、被系统挂起都不会影响房间存活。
//! 频率远低于服务端限流（其他 `/api/v1/*` 为 2 次/秒）。

use super::api::{self, ApiError};
use super::rooms::{fetch_notifications, fetch_rooms};
use super::status;
use crate::state::AppState;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager};

/// 在线心跳：服务端 TTL 90 秒，留一倍余量
const PRESENCE_EVERY: u64 = 45;
/// 通知轮询
const NOTIFY_EVERY: u64 = 15;
/// 好友列表刷新（在线状态由服务端 TTL 计算）
const FRIENDS_EVERY: u64 = 60;
/// 房间目录刷新：房间可能被管理员下架、房主停止分享或心跳超时，
/// 客户端不主动拉就一直显示旧卡片，所以固定周期对账一次。
const ROOMS_EVERY: u64 = 30;
/// 房间心跳：服务端 TTL 120 秒，留三倍余量
const ROOM_EVERY: u64 = 40;

pub fn start_background(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_presence = 0u64;
        let mut last_notify = 0u64;
        let mut last_friends = 0u64;
        let mut last_rooms = 0u64;
        // 上一次推送的房间列表，内容没变就不再打扰前端
        let mut rooms_sig = String::new();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let state = app.state::<AppState>();
            if !state.qookix().logged_in() {
                last_presence = 0;
                last_notify = 0;
                last_friends = 0;
                last_rooms = 0;
                rooms_sig.clear();
                continue;
            }
            let now = crate::util::now_secs();

            if now.saturating_sub(last_presence) >= PRESENCE_EVERY {
                last_presence = now;
                // 顺带汇报「正在玩什么」：好友列表据此显示「在玩 XX 实例」
                let body = current_activity(&state).unwrap_or_else(|| json!({}));
                if let Err(e) = api::post(&state, "/api/v1/presence/heartbeat", body).await {
                    if auth_lost(&app, &state, &e) {
                        continue;
                    }
                }
            }

            if now.saturating_sub(last_notify) >= NOTIFY_EVERY {
                last_notify = now;
                match fetch_notifications(&state).await {
                    Ok(v) => {
                        let has_friend = v
                            .get("friend_requests")
                            .and_then(|a| a.as_array())
                            .is_some_and(|a| !a.is_empty());
                        let has_invite = v
                            .get("room_invites")
                            .and_then(|a| a.as_array())
                            .is_some_and(|a| !a.is_empty());
                        if has_friend || has_invite {
                            let _ = app.emit("qookix://notifications", v);
                        }
                    }
                    Err(e) => {
                        if auth_lost(&app, &state, &e) {
                            continue;
                        }
                    }
                }
            }

            if now.saturating_sub(last_friends) >= FRIENDS_EVERY {
                last_friends = now;
                match api::get(&state, "/api/v1/friends", Vec::new()).await {
                    Ok(v) => {
                        let _ = app.emit("qookix://friends", v);
                    }
                    Err(e) => {
                        auth_lost(&app, &state, &e);
                    }
                }
            }

            if now.saturating_sub(last_rooms) >= ROOMS_EVERY {
                last_rooms = now;
                match fetch_rooms(&state, "friends", 1, 20).await {
                    Ok(v) => {
                        let sig = v.get("rooms").map(|r| r.to_string()).unwrap_or_default();
                        if sig != rooms_sig {
                            rooms_sig = sig;
                            let _ = app.emit("qookix://rooms", v);
                        }
                    }
                    Err(e) => {
                        auth_lost(&app, &state, &e);
                    }
                }
            }
        }
    });
}

/// 令牌彻底失效时通知前端回登录态；返回 true 表示本次已中断
fn auth_lost(app: &tauri::AppHandle, state: &AppState, e: &ApiError) -> bool {
    if e.code != super::CODE_NOT_LOGGED_IN {
        return false;
    }
    let _ = app.emit("qookix://session", status(state));
    true
}

/// 当前在玩的实例 → 心跳里要报的 activity。
///
/// 只在真的跑着游戏时上报；用户关掉「向好友显示我正在玩什么」就彻底不报。
/// 服务器地址（server）刻意不报：那是本机很私密的信息，当前也没有可靠来源。
fn current_activity(state: &AppState) -> Option<serde_json::Value> {
    if !state.settings().share_activity {
        return None;
    }
    // 正常同时只会跑一个实例，取第一个即可
    let inst_id = {
        let pids = state.game_pids.lock().unwrap_or_else(|e| e.into_inner());
        pids.keys().next().cloned()?
    };
    let inst = crate::instances::get_instance(state, &inst_id).ok()?;
    Some(json!({
        "instance": inst.name,
        "mc_version": inst.mc_version,
        "loader": inst.loader.as_str(),
    }))
}

async fn sleep_or_cancel(cancel: &Arc<AtomicBool>, secs: u64) -> bool {
    for _ in 0..secs {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    cancel.load(Ordering::SeqCst)
}

/// 房间分享 supervisor：每 40 秒对一次账，陶瓦那边不在了就关闭目录条目。
pub fn start_share_task(
    app: tauri::AppHandle,
    cancel: Arc<AtomicBool>,
    room_id: String,
    mut code: String,
    title: String,
    visibility: String,
    pack: String,
    mods_count: u32,
) {
    tauri::async_runtime::spawn(async move {
        // 被服务端踢下架/关闭时，把原因带回前端提示一次
        let mut stop_reason: Option<String> = None;
        loop {
            if sleep_or_cancel(&cancel, ROOM_EVERY).await {
                break;
            }
            let state = app.state::<AppState>();
            if !state.qookix().logged_in() {
                break;
            }
            let local = crate::terracotta::local_state(&state).await;
            let Ok(local) = local else {
                // 陶瓦进程没了：目录条目留着只会显示成假房间
                let _ =
                    api::post(&state, "/api/v1/rooms/close", json!({ "room_id": &room_id })).await;
                break;
            };
            let host_ok = local.get("state").and_then(|v| v.as_str()) == Some("host-ok");
            let current = crate::terracotta::room_code_of(&local);
            match current {
                Some(c) if host_ok => {
                    if c != code {
                        // 房间码变了：重新上报，别继续给旧码续命
                        code = c.clone();
                        let mut body = json!({ "code": c, "visibility": &visibility });
                        if !title.is_empty() {
                            body["title"] = json!(&title);
                        }
                        // 清单要一起带上：重报是整条覆盖，不带就等于把环境清单抹了
                        if !pack.is_empty() {
                            body["pack_json"] = json!(&pack);
                            body["mods_count"] = json!(mods_count);
                        }
                        let _ = api::post(&state, "/api/v1/rooms", body).await;
                    } else {
                        // 不传 members：显式传空数组会清掉成员快照
                        match api::post(
                            &state,
                            "/api/v1/rooms/heartbeat",
                            json!({ "room_id": &room_id }),
                        )
                        .await
                        {
                            Ok(_) => {}
                            // 被管理员下架 / 房间已关闭：继续心跳只会一直 409
                            Err(e)
                                if matches!(
                                    e.code.as_str(),
                                    "room_closed"
                                        | "room_removed"
                                        | "room_not_found"
                                        | "not_room_owner"
                                ) =>
                            {
                                stop_reason = Some(e.code);
                                break;
                            }
                            Err(_) => {}
                        }
                    }
                }
                _ => {
                    let _ =
                        api::post(&state, "/api/v1/rooms/close", json!({ "room_id": &room_id }))
                            .await;
                    break;
                }
            }
        }

        // 只有仍是自己这条分享时才清状态，避免把新一轮分享覆盖掉
        let state = app.state::<AppState>();
        let cleared = {
            let mut s = state.qookix();
            let same = s
                .share
                .as_ref()
                .map(|r| r.room_id == room_id)
                .unwrap_or(false);
            if same {
                s.share = None;
            }
            same
        };
        if cleared {
            let _ = app.emit(
                "qookix://share",
                json!({ "sharing": false, "reason": stop_reason }),
            );
        }
    });
}

/// 应用退出时的兜底：关掉目录条目、结束陶瓦进程（顺手补上孤儿进程清理）。
pub fn cleanup_on_exit(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let room_id = {
        let mut s = state.qookix();
        s.share.take().map(|share| {
            share.cancel.store(true, Ordering::SeqCst);
            share.room_id
        })
    };
    let app = app.clone();
    let _ = tauri::async_runtime::block_on(async move {
        let state = app.state::<AppState>();
        if let Some(room_id) = room_id {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                api::post(&state, "/api/v1/rooms/close", json!({ "room_id": room_id })),
            )
            .await;
        }
        let session = state.terracotta.lock().unwrap().take();
        if let Some(s) = session {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &s.pid.to_string(), "/F", "/T"])
                .spawn();
        }
    });
}
