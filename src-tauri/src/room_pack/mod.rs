//! 联机环境清单（P0）+ 差异比对。
//!
//! 清单以**磁盘为准**生成（docs/room-pack-sync-design.md §4.2）：
//! 用户整合包里大量手工文件在 `qookix.json` 里没有登记，只看登记记录会漏。
//!
//! 两条边界：
//! - 清单描述的是**启动器实例**。陶瓦房间真正跑的 Minecraft 由陶瓦自己管理，
//!   它没有暴露游戏目录/mod 清单的接口（实测只有 `/state/*`），所以对陶瓦房间来说这是参照物。
//! - 清单只有几 KB，走服务端；文件本体永远走 P2P，服务端一个字节都不碰。

pub mod net;
pub mod transfer;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager, State};

use crate::models::Instance;
use crate::state::AppState;

/// 清单格式版本
pub const PACK_FORMAT: u32 = 1;

/// 单个文件的在线来源（有来源就不必走 P2P，客人直接重下）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackSource {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
}

/// 清单里的一条文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackFile {
    /// mod | resourcepack | shaderpack
    pub kind: String,
    /// 相对文件夹的文件名（不含目录，杜绝路径穿越）
    pub name: String,
    #[serde(default)]
    pub mod_id: String,
    #[serde(default)]
    pub version: String,
    pub size: u64,
    /// 文件内容 SHA1，比对与传输校验都靠它
    #[serde(default)]
    pub sha1: String,
    /// null = 无在线来源，必须走 P2P
    #[serde(default)]
    pub source: Option<PackSource>,
    /// 资源包 / 光影默认不同步
    #[serde(default)]
    pub optional: bool,
}

/// 一份环境清单
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackManifest {
    pub v: u32,
    #[serde(default)]
    pub mc_version: String,
    #[serde(default)]
    pub loader: String,
    #[serde(default)]
    pub loader_version: String,
    #[serde(default)]
    pub files: Vec<PackFile>,
    /// 需要 P2P 传输的字节数（无在线来源、且非可选）
    #[serde(default)]
    pub need_bytes: u64,
    /// 有在线来源的文件数（客人可自行重下）
    #[serde(default)]
    pub online_count: u32,
}

/// 两份环境的差异
#[derive(Debug, Clone, Serialize, Default)]
pub struct PackDiff {
    /// 房主有、我没有
    pub missing: Vec<PackFile>,
    /// 我有、房主没有
    pub extra: Vec<PackFile>,
    /// 两边都有但内容不同（sha1 不一致，或版本号不同）
    pub version_mismatch: Vec<PackFile>,
    pub mc_version_mismatch: bool,
    pub loader_mismatch: bool,
}

// ------------------------------------------------------------------ 扫描

/// SHA1 缓存：大整合包全量重算很慢，按 size + mtime 判断是否要重算
#[derive(Debug, Default, Serialize, Deserialize)]
struct Sha1Cache {
    #[serde(default)]
    files: HashMap<String, CachedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFile {
    size: u64,
    mtime: u64,
    sha1: String,
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn sha1_file(path: &Path) -> Option<String> {
    use sha1::{Digest, Sha1};
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex(&hasher.finalize()))
}

fn mtime_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn cache_path(state: &AppState, instance_id: &str) -> PathBuf {
    state
        .instances_dir()
        .join(instance_id)
        .join("pack_fingerprint.json")
}

fn load_cache(path: &Path) -> Sha1Cache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_cache(path: &Path, cache: &Sha1Cache) {
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, text);
    }
}

/// 从实例的 `qookix.json` 里按文件名查来源（登记过的才有）
fn source_map(inst: &Instance) -> HashMap<&str, &crate::models::InstalledContent> {
    inst.mods
        .iter()
        .filter(|m| m.enabled)
        .map(|m| (m.filename.as_str(), m))
        .collect()
}

/// 扫描一个目录里的文件，生成清单条目
fn scan_dir(
    dir: &Path,
    kind: &str,
    optional: bool,
    sources: &HashMap<&str, &crate::models::InstalledContent>,
    cache: &mut Sha1Cache,
) -> Vec<PackFile> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else { continue };
        let size = meta.len();
        let mtime = mtime_secs(&meta);
        let cache_key = format!("{kind}/{name}");
        let sha1 = match cache.files.get(&cache_key) {
            Some(c) if c.size == size && c.mtime == mtime => c.sha1.clone(),
            _ => match sha1_file(&path) {
                Some(h) => {
                    cache.files.insert(
                        cache_key,
                        CachedFile {
                            size,
                            mtime,
                            sha1: h.clone(),
                        },
                    );
                    h
                }
                // 读不了就跳过这个文件，宁可清单少一条也不要写进假哈希
                None => continue,
            },
        };
        let rec = sources.get(name);
        out.push(PackFile {
            kind: kind.to_string(),
            name: name.to_string(),
            mod_id: rec.and_then(|r| r.mod_id.clone()).unwrap_or_default(),
            version: rec.and_then(|r| r.version.clone()).unwrap_or_default(),
            size,
            sha1,
            source: rec.and_then(|r| {
                let provider = r.source.clone();
                if provider != "modrinth" && provider != "curseforge" {
                    return None;
                }
                Some(PackSource {
                    provider,
                    project_id: r.project_id.clone()?,
                    version_id: r.version_id.clone()?,
                })
            }),
            optional,
        });
    }
    out
}

/// 生成清单：以磁盘为准，登记记录只用来补 `mod_id` / `version` / 在线来源
pub fn build_manifest(state: &AppState, inst: &Instance) -> PackManifest {
    let base = state.instances_dir().join(&inst.id);
    let cp = cache_path(state, &inst.id);
    let mut cache = load_cache(&cp);
    let sources = source_map(inst);

    let mut files = Vec::new();
    files.extend(scan_dir(
        &base.join("mods"),
        "mod",
        false,
        &sources,
        &mut cache,
    ));
    files.extend(scan_dir(
        &base.join("resourcepacks"),
        "resourcepack",
        true,
        &sources,
        &mut cache,
    ));
    files.extend(scan_dir(
        &base.join("shaderpacks"),
        "shaderpack",
        true,
        &sources,
        &mut cache,
    ));

    save_cache(&cp, &cache);
    // 排序保证同一环境每次生成的清单一致（哈希稳定）
    files.sort_by(|a, b| (a.kind.clone(), a.name.clone()).cmp(&(b.kind.clone(), b.name.clone())));

    let need_bytes: u64 = files
        .iter()
        .filter(|f| f.source.is_none() && !f.optional)
        .map(|f| f.size)
        .sum();
    let online_count = files.iter().filter(|f| f.source.is_some()).count() as u32;

    PackManifest {
        v: PACK_FORMAT,
        mc_version: inst.mc_version.clone(),
        loader: inst.loader.as_str().to_string(),
        loader_version: inst.loader_version.clone().unwrap_or_default(),
        files,
        need_bytes,
        online_count,
    }
}

// ------------------------------------------------------------------ 比对

/// 同一条文件在两份清单里的身份：有 mod_id 用 mod_id，否则用文件名
fn identity(f: &PackFile) -> String {
    if f.mod_id.is_empty() {
        format!("{}:{}", f.kind, f.name)
    } else {
        format!("{}:{}", f.kind, f.mod_id)
    }
}

/// 比对：本地实例 vs 房主清单
pub fn compare(local: &PackManifest, remote: &PackManifest) -> PackDiff {
    let mut diff = PackDiff::default();

    for rf in &remote.files {
        // 可选内容（资源包/光影）不参与「必须补齐」的判断
        if rf.optional {
            continue;
        }
        match local.files.iter().find(|lf| identity(lf) == identity(rf)) {
            None => diff.missing.push(rf.clone()),
            Some(lf) => {
                let same_content = !lf.sha1.is_empty() && lf.sha1 == rf.sha1;
                let same_version = lf.version.is_empty()
                    || rf.version.is_empty()
                    || lf.version == rf.version;
                if !same_content || !same_version {
                    let mut m = rf.clone();
                    if !same_version {
                        m.version = format!("{} → {}", lf.version, rf.version);
                    }
                    diff.version_mismatch.push(m);
                }
            }
        }
    }

    for lf in &local.files {
        if lf.optional {
            continue;
        }
        if !remote.files.iter().any(|rf| identity(rf) == identity(lf)) {
            diff.extra.push(lf.clone());
        }
    }

    diff.mc_version_mismatch = !remote.mc_version.is_empty()
        && !local.mc_version.is_empty()
        && remote.mc_version != local.mc_version;
    diff.loader_mismatch = !remote.loader.is_empty()
        && !local.loader.is_empty()
        && remote.loader != local.loader;
    diff
}

// ------------------------------------------------------------------ 命令层

/// 客户端生成一份清单（房主分享时用）
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackManifestOut {
    /// 清单 JSON（字符串，服务端按字符串存）
    pub manifest: String,
    pub mods_count: usize,
    pub mc_version: String,
    pub loader: String,
    /// 需要 P2P 传输的字节数（无在线来源的 mod）
    pub need_bytes: u64,
}

#[tauri::command]
pub async fn qookix_pack_manifest(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
) -> Result<PackManifestOut, String> {
    let inst = crate::instances::get_instance(&state, &instance_id)?;
    // 扫盘 + 算 SHA1 是重活，扔到阻塞线程池，别占着异步运行时
    let manifest = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        build_manifest(&state, &inst)
    })
    .await
    .map_err(|e| format!("生成清单失败: {e}"))?;

    let mods_count = manifest.files.iter().filter(|f| f.kind == "mod").count();
    let need_bytes = manifest.need_bytes;
    let mc_version = manifest.mc_version.clone();
    let loader = manifest.loader.clone();
    let json = serde_json::to_string(&manifest).map_err(|e| format!("清单序列化失败: {e}"))?;
    Ok(PackManifestOut {
        manifest: json,
        mods_count,
        mc_version,
        loader,
        need_bytes,
    })
}

/// 比对结果
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackCompareOut {
    pub has_pack: bool,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<PackDiff>,
    pub remote_mc_version: String,
    pub remote_loader: String,
    /// 房主清单里的 mod 数量
    pub remote_mods: usize,
    pub local_mods: usize,
    /// 需要从房主拉取的字节数（只算本地缺的部分）
    pub pull_bytes: u64,
    /// 需要在本地删掉 / 忽略的数量
    pub extra_count: usize,
}

/// 客人在加入前比对：拉房间详情里的清单，和本地实例比
#[tauri::command]
pub async fn qookix_compare_pack(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    room_id: String,
    instance_id: String,
) -> Result<PackCompareOut, String> {
    let inst = crate::instances::get_instance(&state, &instance_id)?;

    let data = crate::qookix::api::get(
        &state,
        "/api/v1/rooms/detail",
        vec![("room_id".to_string(), room_id)],
    )
    .await
    .map_err(|e| e.message)?;

    let room = data.get("room").cloned().unwrap_or(serde_json::Value::Null);
    let pack = room.get("pack").cloned().unwrap_or(serde_json::Value::Null);
    if pack.is_null() {
        return Ok(PackCompareOut {
            has_pack: false,
            ok: true,
            diff: None,
            remote_mc_version: String::new(),
            remote_loader: String::new(),
            remote_mods: 0,
            local_mods: inst.mods.iter().filter(|m| m.enabled).count(),
            pull_bytes: 0,
            extra_count: 0,
        });
    }
    let remote: PackManifest =
        serde_json::from_value(pack).map_err(|e| format!("房主的环境清单解析失败: {e}"))?;

    let local = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        build_manifest(&state, &inst)
    })
    .await
    .map_err(|e| format!("读取本地环境失败: {e}"))?;

    let diff = compare(&local, &remote);
    let ok = diff.missing.is_empty()
        && diff.extra.is_empty()
        && diff.version_mismatch.is_empty()
        && !diff.mc_version_mismatch
        && !diff.loader_mismatch;
    // 只统计「真要传」的部分：有在线来源的客人自己能重下，不算房主上行
    let pull_bytes: u64 = diff
        .missing
        .iter()
        .chain(diff.version_mismatch.iter())
        .filter(|f| f.source.is_none())
        .map(|f| f.size)
        .sum();

    Ok(PackCompareOut {
        has_pack: true,
        ok,
        diff: Some(diff),
        remote_mc_version: remote.mc_version,
        remote_loader: remote.loader,
        remote_mods: remote.files.iter().filter(|f| f.kind == "mod").count(),
        local_mods: local.files.iter().filter(|f| f.kind == "mod").count(),
        pull_bytes,
        extra_count: 0,
    })
}

// ------------------------------------------------------------------ 房主侧：开服务

/// 正在共享环境的会话（单例：一个启动器同时只挂一个房间）
struct PackServe {
    /// 中继模式不需要 EasyTier 节点，这里为 None
    node: Option<net::Node>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for PackServe {
    fn drop(&mut self) {
        self.server.abort();
    }
}

static SERVE: std::sync::Mutex<Option<PackServe>> = std::sync::Mutex::new(None);

/// 房主使用固定虚拟地址，客人按连接选择虚拟地址与本地端口。
const HOST_VIP: &str = "10.144.144.1";
const HOST_RPC_PORT: u16 = 16888;
/// 中继模式下文件服务的默认端口（要和 frp 隧道里填的本地端口一致）
const DEFAULT_HTTP_PORT: u16 = 18099;

/// 取一个当前空闲的本机端口
fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    l.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

fn stop_serve_inner() {
    let session = SERVE.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(session);
}

/// 房主开服务：起 HTTP + EasyTier 节点，返回要写进房间元数据的 `pack_net`
#[tauri::command]
pub async fn qookix_pack_serve(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    instance_id: String,
    // 走 frp 中继时房主对外的地址（http://节点:远端端口），留空表示只走局域网
    relay_url: Option<String>,
    // 文件服务端口：中继模式必须固定成隧道里配的本地端口
    port: Option<u16>,
) -> Result<serde_json::Value, String> {
    // 先停掉上一次，避免两个节点抢同一个网络名
    stop_serve_inner();

    let inst = crate::instances::get_instance(&state, &instance_id)?;
    let manifest = {
        let app2 = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let state = app2.state::<AppState>();
            build_manifest(&state, &inst)
        })
        .await
        .map_err(|e| format!("生成清单失败: {e}"))?
    };

    // 一次性 token：只防误连，不承担鉴权（真正的边界是「只服务清单里的文件」）
    let token = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(format!("{}-{}", instance_id, crate::util::now_secs()).as_bytes());
        h.update(&std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos().to_le_bytes())
            .unwrap_or([0; 4]));
        h.finalize()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };

    let base = state.instances_dir().join(&instance_id);
    // 中继模式必须用隧道里写死的本地端口；局域网模式拿随机端口即可
    let want_port = if relay_url.is_some() {
        port.unwrap_or(DEFAULT_HTTP_PORT)
    } else {
        0
    };
    let (http_port, server) =
        transfer::serve(base, manifest.files.clone(), token.clone(), want_port)
            .await
            .map_err(|e| format!("启动文件服务失败: {e}"))?;
    let mut session = PackServe { node: None, server };

    let mods_count = manifest.files.iter().filter(|f| f.kind == "mod").count();
    let manifest_json = serde_json::to_string(&manifest).map_err(|e| e.to_string())?;

    // ① 中继模式：只把本地 HTTP 端口交给 frpc，完全不依赖 EasyTier
    if let Some(url) = relay_url.filter(|u| !u.trim().is_empty()) {
        let pack_net = net::encode_relay(url.trim(), &token);
        {
            let mut guard = SERVE.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(session);
        }
        return Ok(serde_json::json!({
            "packNet": pack_net,
            "manifest": manifest_json,
            "modsCount": mods_count,
            "needBytes": manifest.need_bytes,
            "httpPort": http_port,
            "mode": "relay",
        }));
    }

    // ② 局域网模式：起 EasyTier 节点
    let (core, _cli) = net::locate_easytier().ok_or_else(|| {
        "找不到 EasyTier（请先运行一次陶瓦联机，或改用 frp 隧道模式）".to_string()
    })?;
    // 节点监听必须绑真实网卡 IP：--no-tun 下用 127.0.0.1 会 WSAEADDRNOTAVAIL
    let lan = net::lan_ipv4().ok_or_else(|| "拿不到本机局域网 IP".to_string())?;
    let peer_port = free_port()?;
    let net_info = net::PackNet {
        network: format!("qkx-{}", &token[..8.min(token.len())]),
        secret: token.clone(),
        peer_addr: format!("{lan}:{peer_port}"),
        vip: HOST_VIP.to_string(),
        http_port,
        token: token.clone(),
    };
    let node = net::spawn_node(
        &core,
        &net_info,
        HOST_VIP,
        HOST_RPC_PORT,
        Some(&format!("tcp://{lan}:{peer_port}")),
        None,
        &[http_port],
    )
    .map_err(|e| format!("启动 P2P 节点失败: {e}"))?;
    session.node = Some(node);

    {
        let mut guard = SERVE.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(session);
    }

    Ok(serde_json::json!({
        "packNet": net_info.encode(),
        "manifest": manifest_json,
        "modsCount": mods_count,
        "needBytes": manifest.need_bytes,
        "httpPort": http_port,
        "mode": "lan",
    }))
}

/// 房主关服务
#[tauri::command]
pub fn qookix_pack_stop_serve() -> Result<(), String> {
    stop_serve_inner();
    Ok(())
}

// ------------------------------------------------------------------ 客人侧：拉文件

/// 客人侧：起节点 → port-forward → 逐文件下载缺失内容
///
/// 只有「房主有、我没有、且没有在线来源」的文件才走隧道；
/// 有 `source` 的交给 `install_content` 走 CDN，不占房主上行。
#[tauri::command]
pub async fn qookix_pack_pull(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    room_id: String,
    instance_id: String,
) -> Result<serde_json::Value, String> {
    let inst = crate::instances::get_instance(&state, &instance_id)?;

    // 1. 拉房间详情：清单 + 连接信息
    let data = crate::qookix::api::get(
        &state,
        "/api/v1/rooms/detail",
        vec![("room_id".to_string(), room_id)],
    )
    .await
    .map_err(|e| e.message)?;
    let room = data.get("room").cloned().unwrap_or(serde_json::Value::Null);
    let pack = room.get("pack").cloned().unwrap_or(serde_json::Value::Null);
    let net_raw = room
        .get("pack_net")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if pack.is_null() {
        return Err("房主没有共享环境清单".into());
    }
    let remote: PackManifest =
        serde_json::from_value(pack).map_err(|e| format!("清单解析失败: {e}"))?;
    let route = net::parse_route(&net_raw)
        .ok_or_else(|| "房主没有开文件服务（连接信息为空或版本不兼容）".to_string())?;

    // 2. 本地比对，挑出「必须从房主拉」的文件
    let inst2 = inst.clone();
    let app2 = app.clone();
    let local = tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        build_manifest(&state, &inst2)
    })
    .await
    .map_err(|e| format!("读取本地环境失败: {e}"))?;
    let diff = compare(&local, &remote);
    let todo: Vec<PackFile> = diff
        .missing
        .iter()
        .chain(diff.version_mismatch.iter())
        .filter(|f| f.source.is_none() && !f.optional)
        .cloned()
        .collect();
    if todo.is_empty() {
        return Ok(serde_json::json!({ "downloaded": 0, "failed": 0, "bytes": 0 }));
    }
    let _instance_files = state.jobs.reserve_game_files(&instance_id)?;

    // 3. 建立通道：中继直连，或起 EasyTier 节点 + 端口映射
    let (origin, token, _node) = match route {
        // 房主已经把 HTTP 端口挂到了公网（frp 等），直接拉
        net::PackRoute::Relay { url, token } => (url, token, None),
        net::PackRoute::Lan(net_info) => {
            let (core, cli) = net::locate_easytier()
                .ok_or_else(|| "找不到 EasyTier（请先运行一次陶瓦联机）".to_string())?;
            let (rpc_port, local_port) = guest_ports()?;
            let id = uuid::Uuid::new_v4();
            let [a, b, ..] = *id.as_bytes();
            let subnet = if a == 144 { 145 } else { a };
            let vip = format!("10.144.{subnet}.{}", 1 + u16::from(b) % 254);
            let mut node = net::spawn_node(
                &core,
                &net_info,
                &vip,
                rpc_port,
                None,
                Some(&format!("tcp://{}", net_info.peer_addr)),
                &[],
            )?;

            // 节点要几秒才能和房主握手，失败就重试几次，别一次失败就报错
            let mut forward_err = None;
            for _ in 0..8 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !node.is_running()? {
                    return Err("EasyTier 节点在连接房主前退出".into());
                }
                match net::port_forward(
                    &cli,
                    rpc_port,
                    local_port,
                    &net_info.vip,
                    net_info.http_port,
                ).await {
                    Ok(()) => {
                        forward_err = None;
                        break;
                    }
                    Err(e) => forward_err = Some(e),
                }
            }
            if let Some(e) = forward_err {
                return Err(format!("连不上房主：{e}"));
            }
            (format!("http://127.0.0.1:{local_port}"), net_info.token.clone(), Some(node))
        }
    };

    // 4. 逐个下载
    let base = state.instances_dir().join(&instance_id);
    let client = reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("网络客户端初始化失败: {e}"))?;

    let total = todo.len();
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut bytes = 0u64;
    for (i, f) in todo.iter().enumerate() {
        let _ = app.emit(
            "qookix://pack-progress",
            serde_json::json!({ "index": i, "total": total, "name": f.name }),
        );
        match transfer::download_file(&client, &origin, &token, f, &base, &f.sha1).await {
            Ok(n) => {
                done += 1;
                bytes += n;
            }
            Err(e) => {
                failed += 1;
                crate::util::log_line(&format!("[pack] 下载 {} 失败: {e}", f.name));
            }
        }
    }
    Ok(serde_json::json!({
        "downloaded": done,
        "failed": failed,
        "bytes": bytes,
    }))
}

fn guest_ports() -> Result<(u16, u16), String> {
    let rpc = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok((rpc.local_addr().map_err(|e| e.to_string())?.port(), free_port()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(kind: &str, name: &str, mod_id: &str, ver: &str, sha1: &str, size: u64) -> PackFile {
        PackFile {
            kind: kind.into(),
            name: name.into(),
            mod_id: mod_id.into(),
            version: ver.into(),
            size,
            sha1: sha1.into(),
            source: None,
            optional: false,
        }
    }

    fn manifest(files: Vec<PackFile>) -> PackManifest {
        PackManifest {
            v: PACK_FORMAT,
            mc_version: "1.20.1".into(),
            loader: "fabric".into(),
            loader_version: "0.15.7".into(),
            files,
            need_bytes: 0,
            online_count: 0,
        }
    }

    #[test]
    fn compare_flags_missing_and_content_change() {
        let local = manifest(vec![file("mod", "sodium.jar", "sodium", "0.5.7", "aaaa", 100)]);
        let remote = manifest(vec![
            file("mod", "sodium.jar", "sodium", "0.5.7", "bbbb", 100),
            file("mod", "iris.jar", "iris", "1.7.0", "cccc", 200),
        ]);
        let d = compare(&local, &remote);
        assert_eq!(d.missing.len(), 1);
        assert_eq!(d.missing[0].name, "iris.jar");
        // 版本号一样但内容哈希不同 → 也算不一致（同 mod 换了构建）
        assert_eq!(d.version_mismatch.len(), 1);
        assert_eq!(d.version_mismatch[0].name, "sodium.jar");
        assert!(!d.mc_version_mismatch);
    }

    #[test]
    fn optional_files_are_ignored() {
        let mut rp = file("resourcepack", "pack.zip", "", "", "dddd", 500);
        rp.optional = true;
        let local = manifest(vec![]);
        let remote = manifest(vec![rp]);
        let d = compare(&local, &remote);
        assert!(d.missing.is_empty(), "资源包不该算进必须补齐");
    }

    #[test]
    fn identity_prefers_mod_id_over_filename() {
        // 同一个 mod 两边文件名不同（带版本号），但 mod_id 相同 → 应视为同一条
        let local = manifest(vec![file("mod", "sodium-0.5.7.jar", "sodium", "0.5.7", "aaaa", 1)]);
        let remote = manifest(vec![file("mod", "sodium-0.5.8.jar", "sodium", "0.5.8", "aaaa", 1)]);
        let d = compare(&local, &remote);
        assert!(d.missing.is_empty());
        assert_eq!(d.version_mismatch.len(), 1);
    }
}
