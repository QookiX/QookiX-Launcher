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
    let dir_for_pack = instance_dir.clone();
    let zip_data = match tokio::task::spawn_blocking(move || pack_instance_zip(&dir_for_pack)).await
    {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Pack error: {e}"))
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Pack task join failed: {e}"),
            )
                .into_response();
        }
    };

    let headers = [
        ("content-type", "application/zip"),
        (
            "content-disposition",
            "attachment",
        ),
    ];
    (
        StatusCode::OK,
        headers,
        axum::body::Body::from(zip_data),
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

/// 单实例最大允许上传 10 GB
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024 * 1024;

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
                        error: "实例过大，超过 10 GB 限制".into(),
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

/// 将实例目录打包为 zip 字节缓冲。
///
/// 包含目录下所有文件（mods / saves / config / resourcepacks / shaderpacks /
/// qookix.json / options.txt / servers.dat 等），不包含游戏本体。
fn pack_instance_zip(instance_dir: &StdPath) -> Result<Vec<u8>, String> {
    let cursor = std::io::Cursor::new(Vec::with_capacity(8 * 1024 * 1024));
    let mut zip = zip::ZipWriter::new(cursor);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored); // 局域网传输，不压缩省 CPU

    add_dir_to_zip(&mut zip, instance_dir, "", &opts)?;

    let cursor = zip
        .finish()
        .map_err(|e| format!("zip finish 失败: {e}"))?;
    Ok(cursor.into_inner())
}

/// 递归将目录添加到 zip
fn add_dir_to_zip<W: Write + std::io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    base: &StdPath,
    prefix: &str,
    opts: &zip::write::SimpleFileOptions,
) -> Result<(), String> {
    let entries = std::fs::read_dir(base).map_err(|e| format!("读目录失败: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let zip_name = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };

        if path.is_dir() {
            add_dir_to_zip(zip, &path, &zip_name, opts)?;
        } else {
            zip.start_file(&zip_name, *opts)
                .map_err(|e| format!("zip start_file 失败: {e}"))?;
            let data = std::fs::read(&path).map_err(|e| format!("读文件失败: {e}"))?;
            zip.write_all(&data)
                .map_err(|e| format!("zip write 失败: {e}"))?;
        }
    }
    Ok(())
}
