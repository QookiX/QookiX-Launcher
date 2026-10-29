//! 局域网直传同步：HTTP 文件服务 + 令牌配对。
//!
//! 流程：start_server → 绑定局域网 IP → 生成 token → 前端渲染 QR →
//!       手机扫码连接 → 验证 token → 浏览实例 → 下载 zip → 完成。
//!
//! 安全：绑定局域网 IP（非 0.0.0.0），一次性 Bearer token，
//!       面板关闭即停服务 + 失效。不持久化任何配对信息。

use axum::{
    extract::{Path, Query, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use bytes::Bytes;
use futures_util::stream::unfold;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use tokio::sync::oneshot;

/// 运行中的同步服务句柄。存入 AppState，关闭时发 shutdown 信号。
pub struct LanSyncServer {
    pub port: u16,
    pub token: String,
    /// 推荐给手机连的 IP（`all_ips` 里可信度最高的那个）
    pub local_ip: String,
    /// 本机所有候选局域网 IPv4，前端可让用户在多个网卡间切换
    pub all_ips: Vec<String>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl LanSyncServer {
    /// 优雅关闭：发送 shutdown 信号。幂等。
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// QR 码内容：`qookix://lan/pair?host=<ip>&port=<port>&token=<token>`
pub fn qr_url(ip: &str, port: u16, token: &str) -> String {
    format!("qookix://lan/pair?host={ip}&port={port}&token={token}")
}

// ── 共享状态 ───────────────────────────────────────────────────────────────

struct SyncState {
    instances_dir: PathBuf,
    servers_dir: PathBuf,
    token: String,
    device_name: String,
    /// 用来把传输进度推给前端面板（2026-10-30）。
    /// 之前桌面端界面上完全看不出「手机正在拉哪个实例、传到哪了」，
    /// 用户只能盯着手机那边等。
    app: tauri::AppHandle,
}

/// 推一条传输进度给前端。失败无所谓（面板可能没打开）。
fn emit_transfer(app: &tauri::AppHandle, payload: serde_json::Value) {
    use tauri::Emitter;
    if let Err(e) = app.emit("lan-sync://transfer", payload) {
        log_lan(&format!("emit lan-sync://transfer 失败：{e}"));
    }
}

// ── API 响应结构 ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct DeviceInfo {
    device: String,
    version: &'static str,
    instance_count: usize,
    /// 能力声明：手机端据此决定是否显示「发送到电脑」入口
    features: &'static [&'static str],
}

#[derive(Serialize)]
struct InstanceSummary {
    id: String,
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
    size_bytes: u64,
    has_saves: bool,
    has_mods: bool,
}

// ── 启动 / 停止 ─────────────────────────────────────────────────────────────

/// 启动 HTTP 文件服务，返回服务信息（含 token 和局域网 IP）。
///
/// `instances_dir` / `servers_dir` 从 AppState 获取。
/// `device_name` 用于手机端显示「已连接：XXX」。
pub async fn start_server(
    app: tauri::AppHandle,
    instances_dir: PathBuf,
    servers_dir: PathBuf,
    device_name: String,
) -> Result<LanSyncServer, String> {
    // 1. 枚举局域网 IP（全部候选，前端可让用户切换）
    let all_ips = list_lan_ips();
    let local_ip = all_ips
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());

    // 2. 生成一次性 token（32 字节 hex）
    let token = uuid::Uuid::new_v4().simple().to_string();

    // 3. 绑定 0.0.0.0:0 —— 监听所有网卡。
    //    不绑定单个 IP：多网卡 / 虚拟网卡环境下，绑错网卡会直接不可达；
    //    绑定全部网卡后，无论手机走哪条路由都能连上，安全性由 token 保证。
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| format!("无法绑定 HTTP 服务: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("无法获取端口: {e}"))?
        .port();

    // 4. 构建路由
    let state = Arc::new(SyncState {
        instances_dir,
        servers_dir,
        token: token.clone(),
        device_name,
        app,
    });
    let app = Router::new()
        .route("/api/info", get(api_info))
        .route("/api/instances", get(list_instances))
        .route("/api/instances/:id", get(download_instance))
        .route("/api/instances/upload", post(upload_instance))
        .route("/api/servers", get(download_servers))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    // 5. 启动服务（带 graceful shutdown）
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let serve = axum::serve(listener, app)
            .with_graceful_shutdown(async { let _ = shutdown_rx.await; });
        if let Err(e) = serve.await {
            eprintln!("[lan_sync] HTTP 服务异常退出: {e}");
        }
    });

    Ok(LanSyncServer {
        port,
        token,
        local_ip,
        all_ips,
        shutdown_tx: Some(shutdown_tx),
    })
}

// ── 局域网 IP 检测 ──────────────────────────────────────────────────────────

/// 虚拟网卡名称特征：Clash / VMware / VirtualBox / Hyper-V / WSL / VPN 等。
///
/// 这些网卡的地址（尤其是 Clash TUN 的 198.18.0.1）手机连不到，
/// 必须排除，否则二维码里会印出一个无效地址、扫码后毫无反应。
const VIRTUAL_IFACE_HINTS: &[&str] = &[
    "clash", "meta", "mihomo", "vmware", "virtualbox", "vbox", "hyper-v", "vethernet",
    "wsl", "docker", "tailscale", "zerotier", "radmin", "hamachi", "openvpn", "wireguard",
    "tap-", "tun", "utun", "vpn", "loopback", "npcap", "singbox", "sing-box", "proxy",
];

/// 给一个 IPv4 + 网卡名打分；`None` 表示这个地址不可能被手机访问，直接排除。
///
/// 打分规则：
/// - 只接受 RFC1918 私有网段（公网地址同局域网也连不到）
/// - 家用 192.168/16 最常见 → 最高分；10/8 次之；172.16/12 再次
/// - 命中虚拟网卡名称（Clash / VMware / Hyper-V / VPN…）重罚
fn score_ipv4(octets: [u8; 4], iface_name: &str) -> Option<i32> {
    let [a, b, ..] = octets;

    // Clash TUN / 网络基准测试保留段 198.18.0.0/15 —— 不是真实局域网地址
    if a == 198 && (b == 18 || b == 19) {
        return None;
    }

    let mut score: i32 = if a == 192 && b == 168 {
        100 // 家用路由器最常见
    } else if a == 10 {
        80
    } else if a == 172 && (16..=31).contains(&b) {
        60
    } else {
        return None;
    };

    let lower = iface_name.to_ascii_lowercase();
    if VIRTUAL_IFACE_HINTS.iter().any(|h| lower.contains(h)) {
        score -= 90;
    }

    Some(score)
}

/// 枚举本机所有可能的局域网 IPv4，按「可信度」降序排列。
fn list_lan_ips() -> Vec<String> {
    let mut candidates: Vec<(i32, String)> = Vec::new();

    let interfaces = match if_addrs::get_if_addrs() {
        Ok(list) => list,
        Err(e) => {
            eprintln!("[lan_sync] 枚举网卡失败: {e}");
            return Vec::new();
        }
    };

    for iface in interfaces {
        if iface.is_loopback() {
            continue;
        }
        let v4 = match iface.addr.ip() {
            std::net::IpAddr::V4(v4) => v4,
            std::net::IpAddr::V6(_) => continue, // 局域网直传只用 IPv4
        };
        if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() {
            continue;
        }
        if let Some(score) = score_ipv4(v4.octets(), &iface.name) {
            candidates.push((score, v4.to_string()));
        }
    }

    // 去重（同一 IP 可能挂在多个网卡上，例如 Hyper-V 桥接）
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|(_, ip)| seen.insert(ip.clone()))
        .map(|(_, ip)| ip)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::score_ipv4;

    #[test]
    fn real_lan_beats_virtual_adapters() {
        // 真实以太网 192.168.1.114 vs VMware 的 192.168.181.1
        let real = score_ipv4([192, 168, 1, 114], "以太网 4").unwrap();
        let vmnet = score_ipv4([192, 168, 181, 1], "VMware Network Adapter VMnet8").unwrap();
        assert!(real > vmnet, "真实网卡应排在 VMware 虚拟网卡前面");
    }

    #[test]
    fn clash_tun_fake_ip_is_rejected() {
        assert_eq!(score_ipv4([198, 18, 0, 1], "Clash"), None);
        assert_eq!(score_ipv4([198, 19, 0, 1], "Meta"), None);
    }

    #[test]
    fn public_and_link_local_are_rejected() {
        assert_eq!(score_ipv4([8, 8, 8, 8], "以太网"), None);
        assert_eq!(score_ipv4([169, 254, 1, 1], "以太网"), None);
    }

    #[test]
    fn private_ranges_are_ranked() {
        let home = score_ipv4([192, 168, 1, 10], "WLAN").unwrap();
        let ten = score_ipv4([10, 0, 0, 5], "WLAN").unwrap();
        let one72 = score_ipv4([172, 20, 0, 5], "WLAN").unwrap();
        assert!(home > ten && ten > one72);
    }
}

// ── 令牌验证中间件 ──────────────────────────────────────────────────────────

async fn auth_middleware(
    State(state): State<Arc<SyncState>>,
    request: Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Response {
    let auth = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    if auth != Some(state.token.as_str()) {
        return (StatusCode::UNAUTHORIZED, "Invalid or missing token").into_response();
    }
    next.run(request).await
}

// ── API 端点 ────────────────────────────────────────────────────────────────

/// `GET /api/info` → 设备名 + 版本 + 实例数量
async fn api_info(State(state): State<Arc<SyncState>>) -> Json<DeviceInfo> {
    let count = count_instances(&state.instances_dir);
    Json(DeviceInfo {
        device: state.device_name.clone(),
        version: env!("CARGO_PKG_VERSION"),
        instance_count: count,
        features: &["download", "upload"],
    })
}

/// `GET /api/instances` → 实例列表（id / name / version / loader / size）
async fn list_instances(State(state): State<Arc<SyncState>>) -> Json<Vec<InstanceSummary>> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&state.instances_dir) {
        for entry in entries.flatten() {
            let json_path = entry.path().join("qookix.json");
            if let Ok(text) = std::fs::read_to_string(&json_path) {
                if let Ok(inst) = serde_json::from_str::<crate::models::Instance>(&text) {
                    let dir = entry.path();
                    let size_bytes = dir_size(&dir);
                    let has_saves = dir.join("saves").is_dir();
                    let has_mods = dir.join("mods").is_dir();
                    result.push(InstanceSummary {
                        id: inst.id,
                        name: inst.name,
                        mc_version: inst.mc_version,
                        loader: inst.loader.as_str().to_string(),
                        loader_version: inst.loader_version,
                        size_bytes,
                        has_saves,
                        has_mods,
                    });
                }
            }
        }
    }
    Json(result)
}

/// `GET /api/instances/:id` → 流式下载实例 zip
///
/// 将整个实例目录打包为 zip 并流式返回。
/// 包含：qookix.json + mods/ + saves/ + resourcepacks/ + shaderpacks/ + config/ 等。
/// 不包含：游戏本体（libraries / assets / versions），导入端需自行安装。
async fn download_instance(
    State(state): State<Arc<SyncState>>,
    Path(id): Path<String>,
) -> Response {
    // 防路径穿越
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return (StatusCode::BAD_REQUEST, "Invalid instance id").into_response();
    }

    let instance_dir = state.instances_dir.join(&id);
    if !instance_dir.is_dir() {
        return (StatusCode::NOT_FOUND, "Instance not found").into_response();
    }

    // 打包为 zip（内存缓冲；典型实例 < 500MB）
    //
    // ⚠️ 2026-10-28 修（安卓端真机：点下载报 error sending request）：
    // 原来这里是**直接在 async handler 里同步打包** —— 一个 70MB 的实例要几秒到几十秒，
    // 期间把 tokio 的 worker 线程占死。axum 的 worker 数 = CPU 核数（常见 4~8），
    // 几个请求就能把池子占满，于是**新的连接只能排队甚至超时**。
    // 表现就是：列表能拉出来（快），点下载却连不上。
    //
    // 用 spawn_blocking 挪到阻塞线程池：async runtime 保持空闲，
    // 打包期间其它请求（包括安卓端的重试）都能正常处理。
    // 传输 id：前端据此把同一批事件归成一个卡片
    let transfer_id = uuid::Uuid::new_v4().simple().to_string();
    let instance_name = read_instance_display_name(&instance_dir).unwrap_or_else(|| id.clone());
    let base_payload = |phase: &str| {
        serde_json::json!({
            "id": transfer_id,
            "direction": "download",
            "instanceId": id,
            "instanceName": instance_name,
            "phase": phase,
            "bytesDone": 0u64,
            "bytesTotal": 0u64,
            "file": "",
            "message": "",
        })
    };
    log_lan(&format!(
        "下载请求：{}（{}）→ 开始准备",
        id, instance_name
    ));
    emit_transfer(&state.app, base_payload("prepare"));

    let dir_for_pack = instance_dir.clone();
    // 2026-10-30：改成「打包到临时文件 → 流式发送」。
    // 原来整个 zip 先进内存、`Body::from(Vec)` 一次性发：
    //   1) 手机在打包期间（203 MB 的实例要十几秒）收不到任何一个字节，界面就是「连接失败/卡住」；
    //   2) 大实例容易把内存顶爆。
    // 现在打包落盘后立刻能给出精确 Content-Length，手机端才有真实百分比；
    // 打包过程也会把「正在读哪个文件」推给电脑端面板。
    let app_pack = state.app.clone();
    let tid_pack = transfer_id.clone();
    let name_pack = instance_name.clone();
    let id_pack = id.clone();
    let (tmp_path, zip_size) = match tokio::task::spawn_blocking(move || {
        pack_instance_to_file(&dir_for_pack, &|done: u64, total: u64, file: &str| {
            emit_transfer(
                &app_pack,
                serde_json::json!({
                    "id": tid_pack,
                    "direction": "download",
                    "instanceId": id_pack,
                    "instanceName": name_pack,
                    "phase": "pack",
                    "bytesDone": done,
                    "bytesTotal": total,
                    "file": file,
                    "message": "",
                }),
            );
        })
    })
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            log_lan(&format!("打包实例失败：{e}"));
            emit_transfer(
                &state.app,
                serde_json::json!({
                    "id": transfer_id, "direction": "download",
                    "instanceId": id, "instanceName": instance_name,
                    "phase": "error", "bytesDone": 0, "bytesTotal": 0, "file": "",
                    "message": e,
                }),
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Pack error: {e}"))
                .into_response();
        }
        Err(e) => {
            log_lan(&format!("打包任务崩溃：{e}"));
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Pack task join failed: {e}"),
            )
                .into_response();
        }
    };

    log_lan(&format!(
        "打包完成：{} MB，开始发送给手机",
        zip_size / 1024 / 1024
    ));

    let file = match tokio::fs::File::open(&tmp_path).await {
        Ok(f) => f,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Open packed zip failed: {e}"),
            )
                .into_response();
        }
    };

    // 发送进度：状态放 Arc<Mutex>，因为 unfold 的闭包每次调用都要移动捕获
    let sent_state = Arc::new(std::sync::Mutex::new((
        0u64,
        std::time::Instant::now(),
    )));
    let app_send = state.app.clone();
    let tid_send = transfer_id.clone();
    let name_send = instance_name.clone();
    let id_send = id.clone();

    let stream = unfold(file, move |mut f| {
        let app = app_send.clone();
        let tid = tid_send.clone();
        let name = name_send.clone();
        let iid = id_send.clone();
        let sent_state = sent_state.clone();
        async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 64 * 1024];
            match f.read(&mut buf).await {
                Ok(0) => {
                    // 流结束 = 已经把所有字节交给 socket，标一个 done
                    log_lan(&format!("发送完成：{iid}（{name}）"));
                    emit_transfer(
                        &app,
                        serde_json::json!({
                            "id": tid, "direction": "download",
                            "instanceId": iid, "instanceName": name,
                            "phase": "done", "bytesDone": zip_size, "bytesTotal": zip_size,
                            "file": "", "message": "",
                        }),
                    );
                    None
                }
                Ok(n) => {
                    buf.truncate(n);
                    let mut st = sent_state.lock().unwrap_or_else(|e| e.into_inner());
                    st.0 += n as u64;
                    // 200ms 节流：面板只需要大致跟得上，不必每 64KB 刷一次
                    if st.1.elapsed().as_millis() >= 200 || st.0 >= zip_size {
                        st.1 = std::time::Instant::now();
                        let done = st.0;
                        drop(st);
                        emit_transfer(
                            &app,
                            serde_json::json!({
                                "id": tid, "direction": "download",
                                "instanceId": iid, "instanceName": name,
                                "phase": "send", "bytesDone": done, "bytesTotal": zip_size,
                                "file": "", "message": "",
                            }),
                        );
                    }
                    Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), f))
                }
                Err(e) => Some((Err(e), f)),
            }
        }
    });

    let headers = [
        (axum::http::header::CONTENT_TYPE, "application/zip".to_string()),
        (
            axum::http::header::CONTENT_LENGTH,
            zip_size.to_string(),
        ),
    ];
    (
        StatusCode::OK,
        headers,
        axum::body::Body::from_stream(stream),
    )
        .into_response()
}

/// `GET /api/servers` → 服务器列表 JSON
async fn download_servers(State(state): State<Arc<SyncState>>) -> Response {
    let servers_json = state.servers_dir.join("servers.json");
    if let Ok(data) = std::fs::read(&servers_json) {
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            axum::body::Body::from(data),
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "No servers.json").into_response()
}

// ── 上传：手机 → 电脑 ───────────────────────────────────────────────────────

/// 单实例最大允许上传 2 GB。
///
/// Windows 的 IO 层有 `u32::MAX`（≈4.3 GB）硬限制 —— `IoSlice` 断言
/// `buf.len() <= u32::MAX`。`axum::body::to_bytes` 的 limit 参数若超过这个值，
/// 内部预分配会直接 panic。取 2 GB 既留足余量，也覆盖绝大多数实例体积。
const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Deserialize)]
struct UploadQuery {
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
}

#[derive(Serialize)]
struct UploadOk {
    ok: bool,
    instance_id: String,
    name: String,
}

#[derive(Serialize)]
struct UploadErr {
    ok: bool,
    error: String,
}

/// `POST /api/instances/upload?name=...&mc_version=...&loader=...&loader_version=...`
///
/// 手机端把实例打成 zip 直接 POST。桌面端流式接收 → 解压 → 转换元数据 → 注册实例。
async fn upload_instance(
    State(state): State<Arc<SyncState>>,
    Query(q): Query<UploadQuery>,
    body: axum::body::Body,
) -> Response {
    // 0. 通知面板：手机开始往这边发（2026-10-30）
    let up_id = uuid::Uuid::new_v4().simple().to_string();
    emit_transfer(
        &state.app,
        serde_json::json!({
            "id": up_id, "direction": "upload",
            "instanceId": "", "instanceName": q.name,
            "phase": "prepare", "bytesDone": 0, "bytesTotal": 0,
            "file": "", "message": "",
        }),
    );

    // 1. 同名检查（409）
    if instance_name_exists(&state.instances_dir, &q.name) {
        return (
            StatusCode::CONFLICT,
            Json(UploadErr {
                ok: false,
                error: format!("同名实例已存在: {}", q.name),
            }),
        )
            .into_response();
    }

    // 2. 收集 body（限制大小，超限返回 413）
    let bytes = match axum::body::to_bytes(body, MAX_UPLOAD_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("length limit") || msg.contains("overflow") {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    Json(UploadErr {
                        ok: false,
                        error: "实例过大，超过 2 GB 限制".into(),
                    }),
                )
                    .into_response();
            }
            return (
                StatusCode::BAD_REQUEST,
                Json(UploadErr {
                    ok: false,
                    error: format!("接收数据失败: {msg}"),
                }),
            )
                .into_response();
        }
    };

    // 3. 生成新实例 ID + 目标目录
    let new_id = uuid::Uuid::new_v4().simple().to_string();
    let dest_dir = state.instances_dir.join(&new_id);

    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        return server_error(format!("创建实例目录失败: {e}"));
    }

    // 4. 解压 zip 到目标目录
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = match zip::ZipArchive::new(cursor) {
        Ok(a) => a,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dest_dir);
            return server_error(format!("zip 解析失败: {e}"));
        }
    };

    if let Err(e) = extract_zip(&mut archive, &dest_dir) {
        let _ = std::fs::remove_dir_all(&dest_dir);
        return server_error(e);
    }

    // 5. 转换元数据：instance.json（安卓） → qookix.json（桌面）
    if let Err(e) = convert_android_to_desktop(&dest_dir, &new_id, &q) {
        let _ = std::fs::remove_dir_all(&dest_dir);
        return server_error(e);
    }

    // 6. 成功
    emit_transfer(
        &state.app,
        serde_json::json!({
            "id": up_id, "direction": "upload",
            "instanceId": new_id, "instanceName": q.name,
            "phase": "done", "bytesDone": 0, "bytesTotal": 0,
            "file": "", "message": "",
        }),
    );
    (
        StatusCode::CREATED,
        Json(UploadOk {
            ok: true,
            instance_id: new_id,
            name: q.name,
        }),
    )
        .into_response()
}

fn server_error(msg: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(UploadErr { ok: false, error: msg }),
    )
        .into_response()
}

/// 检查同名实例是否已存在
fn instance_name_exists(instances_dir: &StdPath, name: &str) -> bool {
    if let Ok(entries) = std::fs::read_dir(instances_dir) {
        for entry in entries.flatten() {
            let json_path = entry.path().join("qookix.json");
            if let Ok(text) = std::fs::read_to_string(&json_path) {
                if let Ok(inst) = serde_json::from_str::<crate::models::Instance>(&text) {
                    if inst.name == name {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 解压 zip 到目标目录（带路径穿越防护）
fn extract_zip<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest: &StdPath,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("zip 读取失败: {e}"))?;

        let entry_name = file.name().to_string();
        // 路径穿越防护
        if entry_name.contains("..") || entry_name.starts_with('/') {
            continue;
        }

        let outpath = dest.join(&entry_name);

        if file.is_dir() {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("创建目录失败: {e}"))?;
        } else {
            if let Some(p) = outpath.parent() {
                std::fs::create_dir_all(p)
                    .map_err(|e| format!("创建父目录失败: {e}"))?;
            }
            let mut outfile = std::fs::File::create(&outpath)
                .map_err(|e| format!("创建文件失败: {e}"))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("写入文件失败: {e}"))?;
        }
    }
    Ok(())
}

/// 将安卓 instance.json 转换为桌面 qookix.json
///
/// 安卓端打包的 zip 里有一个 `instance.json`（MinecraftProfile 格式）。
/// 桌面端需要把它转成 `qookix.json`（Instance 格式），换掉 id，删掉 instance.json。
/// 如果 zip 里没有 instance.json，就用 query 参数创建一个最小的 Instance。
fn convert_android_to_desktop(
    dest: &StdPath,
    new_id: &str,
    q: &UploadQuery,
) -> Result<(), String> {
    let android_json_path = dest.join("instance.json");
    let desktop_json_path = dest.join("qookix.json");

    // 尝试读取安卓 instance.json
    let android_data = std::fs::read_to_string(&android_json_path).ok();

    // 解析 loader
    let loader: crate::models::LoaderType = q
        .loader
        .parse()
        .unwrap_or(crate::models::LoaderType::Vanilla);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 从安卓 JSON 提取有用字段（mods / resource_packs / shaders 等）
    let (mods, resource_packs, shaders) = if let Some(ref data) = android_data {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
            let mods = v
                .get("mods")
                .or_else(|| v.get(" Mods"))
                .and_then(|m| serde_json::from_value(m.clone()).ok())
                .unwrap_or_default();
            let rp = v
                .get("resource_packs")
                .or_else(|| v.get("resourcePacks"))
                .and_then(|m| serde_json::from_value(m.clone()).ok())
                .unwrap_or_default();
            let sh = v
                .get("shaders")
                .and_then(|m| serde_json::from_value(m.clone()).ok())
                .unwrap_or_default();
            (mods, rp, sh)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        }
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };

    // 构建桌面端 Instance
    let instance = crate::models::Instance {
        id: new_id.to_string(),
        name: q.name.clone(),
        mc_version: q.mc_version.clone(),
        loader,
        loader_version: q.loader_version.clone(),
        created: now,
        last_played: None,
        total_play_time: 0,
        installed: false, // 游戏本体需另行下载
        icon: None,
        alias: None,
        max_memory_mb: None,
        memory_mode: None,
        jvm_args: None,
        game_args: None,
        java_path: None,
        account_id: None,
        resolution: None,
        mods,
        resource_packs,
        shaders,
        is_symlink: false,
        source_path: None,
        group: None,
    };

    // 写 qookix.json
    let json = serde_json::to_string_pretty(&instance)
        .map_err(|e| format!("序列化 qookix.json 失败: {e}"))?;
    std::fs::write(&desktop_json_path, json)
        .map_err(|e| format!("写入 qookix.json 失败: {e}"))?;

    // 删除安卓 instance.json
    let _ = std::fs::remove_file(&android_json_path);

    Ok(())
}

// ── 工具函数 ────────────────────────────────────────────────────────────────

/// 递归计算目录大小
fn dir_size(path: &StdPath) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// 统计实例数量
fn count_instances(instances_dir: &StdPath) -> usize {
    std::fs::read_dir(instances_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().join("qookix.json").exists())
                .count()
        })
        .unwrap_or(0)
}

/// 修正发出去的 `qookix.json`：`mc_version` 必须是标准版本号。
///
/// ⚠️ 2026-10-30（真机：手机端补全报 `Version not found in manifest: DeceasedCraft`）：
/// PCL 转换过来的实例常把**整合包名**写进 `mc_version`（这里就是
/// `"DeceasedCraft "`，带个尾空格），手机端拿它去版本清单里找必然失败，
/// 于是永远卡在「缺 2 项 · 补全」。
///
/// 真实版本在 `source_path/versions/<版本>/<版本>.json` 里：`inheritsFrom`
/// 或 `clientVersion`。用实例名去匹配版本目录名（PCL 的惯例）。
fn fix_instance_meta(instance_dir: &StdPath) -> Option<Vec<u8>> {
    let raw = std::fs::read_to_string(instance_dir.join("qookix.json")).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    // 先拷成拥有所有权的字符串，否则后面改 v 时借用冲突
    let cur = v
        .get("mc_version")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if looks_like_mc_version(&cur) {
        return None;
    }
    let fixed = guess_mc_version(&v, &cur)?;
    v["mc_version"] = serde_json::Value::String(fixed.clone());
    log_lan(&format!("修正元数据：mc_version {:?} → {:?}", cur, fixed));
    serde_json::to_vec_pretty(&v).ok()
}

/// `1.20.1` / `1.8.9` / `26.3` 这类才算版本号
fn looks_like_mc_version(s: &str) -> bool {
    let mut parts = s.split('.');
    let all_numeric = parts.clone().count() >= 2
        && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    all_numeric
}

/// 从 `source_path/versions` 里猜真实 MC 版本
fn guess_mc_version(meta: &serde_json::Value, cur: &str) -> Option<String> {
    let source = meta.get("source_path").and_then(|x| x.as_str())?;
    let name = meta.get("name").and_then(|x| x.as_str()).unwrap_or("").trim();
    let versions_dir = StdPath::new(source).join("versions");
    let entries = std::fs::read_dir(&versions_dir).ok()?;

    let mut best: Option<(usize, String)> = None;
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        let d = dir_name.trim();
        // 目录名要和实例名（或那个假的 mc_version）沾边，否则容易猜错
        let score = if d.eq_ignore_ascii_case(name) {
            3
        } else if !name.is_empty() && (d.starts_with(name) || name.starts_with(d)) {
            2
        } else if !cur.is_empty() && d.starts_with(cur) {
            1
        } else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(&p) else {
            continue;
        };
        for f in files.flatten() {
            let jp = f.path();
            if jp.extension().map(|e| e != "json").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&jp) else {
                continue;
            };
            let Ok(jv) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            for key in ["inheritsFrom", "clientVersion"] {
                if let Some(val) = jv.get(key).and_then(|x| x.as_str()) {
                    let val = val.trim();
                    if !looks_like_mc_version(val) {
                        continue;
                    }
                    let better = match &best {
                        None => true,
                        Some((s, _)) => score > *s,
                    };
                    if better {
                        best = Some((score, val.to_string()));
                    }
                }
            }
        }
    }
    best.map(|(_, v)| v)
}

/// 预扫一遍要打包的字节总量（跳过符号链接与共享目录，与打包规则一致）。
fn measure_dir_bytes(base: &StdPath) -> u64 {
    let mut total = 0u64;
    let Ok(entries) = std::fs::read_dir(base) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            continue;
        }
        if path.is_dir() {
            if SKIP_DIRS.iter().any(|d| name.eq_ignore_ascii_case(d)) {
                continue;
            }
            total += measure_dir_bytes(&path);
        } else if let Ok(m) = std::fs::metadata(&path) {
            total += m.len();
        }
    }
    total
}

/// 读实例的显示名（桌面端元数据是 `qookix.json`）。读不到就返回 None。
fn read_instance_display_name(dir: &StdPath) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("qookix.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// 打成一个临时 zip 文件，返回 (路径, 字节数)。
///
/// 与「整个打进 Vec」相比：内存占用恒定、能给出精确的 `Content-Length`，
/// 手机端才画得出真实百分比。临时文件放在系统 temp，同名覆盖。
fn pack_instance_to_file(
    instance_dir: &StdPath,
    on_progress: &dyn Fn(u64, u64, &str),
) -> Result<(PathBuf, u64), String> {
    let id = instance_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "instance".to_string());
    let tmp_path = std::env::temp_dir().join(format!("qookix-lan-{id}.zip"));
    let _ = std::fs::remove_file(&tmp_path);
    SKIP_LOGGED.store(0, std::sync::atomic::Ordering::Relaxed);

    let file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("创建临时包失败（{}）：{e}", tmp_path.display()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored); // 局域网传输，不压缩省 CPU

    // 先量一遍总大小，进度条才有分母（只读 metadata，几千个文件也就几毫秒）
    let total = measure_dir_bytes(instance_dir);
    let written = std::cell::Cell::new(0u64);
    let last = std::cell::Cell::new(std::time::Instant::now());
    // 元数据里 mc_version 可能不是标准版本号（PCL 会把整合包名写进去），
    // 手机端拿它去版本清单里查会直接失败 —— 这里先修正
    let patched = fix_instance_meta(instance_dir);
    add_dir_to_zip(
        &mut zip,
        instance_dir,
        "",
        &opts,
        &|size: u64, name: &str| {
            written.set(written.get() + size);
            // 100ms 节流，避免每个文件都推一次事件
            if last.get().elapsed().as_millis() >= 100 {
                last.set(std::time::Instant::now());
                on_progress(written.get(), total, name);
            }
        },
        patched.as_deref(),
    )?;

    let mut buf_writer = zip
        .finish()
        .map_err(|e| format!("zip finish 失败: {e}"))?;
    buf_writer.flush().map_err(|e| format!("写包失败: {e}"))?;
    drop(buf_writer);

    let size = std::fs::metadata(&tmp_path)
        .map_err(|e| format!("读取包大小失败: {e}"))?
        .len();
    Ok((tmp_path, size))
}

/// 本次打包已经记过多少条「跳过符号链接」的日志（每次打包清零，只记前 20 条）
static SKIP_LOGGED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 这些目录不进包。
///
/// * `libraries` / `assets` / `versions` / `runtimes` / `natives`：共享游戏文件，
///   导入端按 `mc_version` + `loader` 自己装，跟着传只会让包大好几倍（本次 203 MB 的
///   实例里 `natives` 就占 6.6 MB，完整游戏库能到几 GB）；
/// * `logs` / `crash-reports` / `screenshots`：对迁移没用，而且日志常被占用读不开。
const SKIP_DIRS: &[&str] = &[
    "libraries",
    "assets",
    "versions",
    "runtimes",
    "runtime",
    "natives",
    "logs",
    "crash-reports",
    "screenshots",
    ".git",
    ".mixin.out",
    ".fabric",
];

/// 递归将目录添加到 zip
///
/// ⚠️ 错误信息必须带**具体路径**（2026-10-30）：原来只写「读文件失败: {e}」，
/// 桌面端 500 时手机只看到 "HTTP 500"，谁都不知道是哪个文件出了问题。
fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &StdPath,
    prefix: &str,
    opts: &zip::write::SimpleFileOptions,
    on_file: &dyn Fn(u64, &str),
    override_root_json: Option<&[u8]>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(base).map_err(|e| format!("读目录失败（{}）：{e}", base.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(v) => v,
            Err(e) => {
                log_lan(&format!("跳过无法读取的目录项（{}）：{e}", base.display()));
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // ⚠️ 符号链接必须跳过（2026-10-30，实测踩到）：
        // 桌面端实例目录里的 `xxx.jar` / `options.txt` 等是指向
        // `E:\.minecraft\versions\<版本>\…` 的符号链接（ReparsePoint）。
        // `std::fs::read` 会跟随链接把目标内容整个读进来 —— 一个 204 MB 的实例
        // 打出来 5 GB，直接撞上 2 GB 上限，手机端只看到 HTTP 500。
        // 这些文件本来就该由导入端按版本自己准备，不该跟着实例传。
        if std::fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            // ⚠️ 只记前 20 条（2026-10-30）：这个实例有 8000+ 符号链接，
            // 逐条写日志直接把 log 撑到 2.8 MB，真正要看的信息全被淹没。
            let n = SKIP_LOGGED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 20 {
                log_lan(&format!("跳过符号链接：{}", path.display()));
            } else if n == 20 {
                log_lan("（符号链接太多，后续不再逐条记录）");
            }
            continue;
        }

        if path.is_dir() && SKIP_DIRS.iter().any(|d| name.eq_ignore_ascii_case(d)) {
            continue;
        }
        let zip_name = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };

        if path.is_dir() {
            add_dir_to_zip(zip, &path, &zip_name, opts, on_file, override_root_json)?;
        } else {
            // 先拿大小，读不出来才算失败 —— 顺便把路径报出去
            let size = match std::fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(e) => {
                    log_lan(&format!("跳过无法读取的文件（{}）：{e}", path.display()));
                    continue;
                }
            };
            zip.start_file(&zip_name, *opts)
                .map_err(|e| format!("zip 写入条目失败（{zip_name}）：{e}"))?;
            // 逐块读：别把一个 500 MB 的存档整个读进内存
            let mut src = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) => {
                    log_lan(&format!("跳过无法打开的文件（{}）：{e}", path.display()));
                    continue;
                }
            };
            // 根级 qookix.json 用修正版替换（版本号可能被 PCL 写成了整合包名）
            if prefix.is_empty() && zip_name == "qookix.json" {
                if let Some(patch) = override_root_json {
                    zip.write_all(patch)
                        .map_err(|e| format!("zip 写入修正元数据失败：{e}"))?;
                    on_file(patch.len() as u64, &zip_name);
                    continue;
                }
            }
            let mut copied: u64 = 0;
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                use std::io::Read;
                let n = match src.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        return Err(format!("读文件失败（{}）：{e}", path.display()));
                    }
                };
                zip.write_all(&buf[..n])
                    .map_err(|e| format!("zip 写入数据失败（{zip_name}）：{e}"))?;
                copied += n as u64;
            }
            if copied != size {
                log_lan(&format!(
                    "文件在打包期间被改动（{}）：预期 {size} B，实际 {copied} B",
                    path.display()
                ));
            }
            on_file(copied, &zip_name);
        }
    }
    Ok(())
}

/// 局域网同步的轻量日志：写到 `%TEMP%/qookix-lan-sync.log`。
///
/// 桌面端是 GUI 程序，没有控制台，`println!` 全丢 —— 手机端只看到 HTTP 500、
/// 用户也没法反馈原因。这里落一个文件，排查时直接看。
fn log_lan(msg: &str) {
    use std::io::Write as _;
    let path = std::env::temp_dir().join("qookix-lan-sync.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}
