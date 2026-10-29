//! 局域网同步 Tauri 命令。

use crate::lan_sync;
use crate::state::AppState;
use serde_json::json;
use tauri::State;

/// 启动局域网同步服务，返回 { ip, port, token, qrUrl, ips }。
///
/// 前端拿到 qrUrl 后用 qrcode JS 库渲染二维码；
/// `ips` 是本机所有候选局域网地址，多网卡时可让用户切换。
#[tauri::command]
pub async fn lan_sync_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    // 已在运行？先关旧的
    {
        let mut guard = state.lan_sync.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref mut srv) = *guard {
            srv.shutdown();
        }
        *guard = None;
    }

    let device_name = hostname();
    let server = lan_sync::start_server(
        app,
        state.instances_dir(),
        state.servers_dir(),
        device_name,
    )
    .await?;

    let qr_url = lan_sync::qr_url(&server.local_ip, server.port, &server.token);
    let resp = json!({
        "ip": server.local_ip,
        "port": server.port,
        "token": server.token,
        "qrUrl": qr_url,
        "ips": server.all_ips,
    });

    let mut guard = state.lan_sync.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(server);

    Ok(resp)
}

/// 关闭局域网同步服务。
#[tauri::command]
pub fn lan_sync_stop(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.lan_sync.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref mut srv) = *guard {
        srv.shutdown();
    }
    *guard = None;
    Ok(())
}

/// 查询当前服务状态。
#[tauri::command]
pub fn lan_sync_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let guard = state.lan_sync.lock().unwrap_or_else(|e| e.into_inner());
    match &*guard {
        Some(srv) => Ok(json!({
            "running": true,
            "ip": srv.local_ip,
            "port": srv.port,
            "qrUrl": lan_sync::qr_url(&srv.local_ip, srv.port, &srv.token),
            "ips": srv.all_ips,
        })),
        None => Ok(json!({ "running": false })),
    }
}

/// 切换使用的局域网 IP（多网卡时用户手动选择），同时重建二维码内容。
#[tauri::command]
pub fn lan_sync_pick_ip(state: State<'_, AppState>, ip: String) -> Result<serde_json::Value, String> {
    let mut guard = state.lan_sync.lock().unwrap_or_else(|e| e.into_inner());
    let srv = guard
        .as_mut()
        .ok_or_else(|| "局域网同步服务未运行".to_string())?;
    if !srv.all_ips.iter().any(|x| x == &ip) {
        return Err(format!("未知的局域网地址: {ip}"));
    }
    srv.local_ip = ip;
    Ok(json!({
        "running": true,
        "ip": srv.local_ip,
        "port": srv.port,
        "qrUrl": lan_sync::qr_url(&srv.local_ip, srv.port, &srv.token),
        "ips": srv.all_ips,
    }))
}

/// 获取本机主机名（用于手机端显示「已连接：XXX」）。
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "QookiX PC".to_string())
}
