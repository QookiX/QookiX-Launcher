//! 单文件 HTTP 服务（房主侧）与分片下载器（客人侧）。
//!
//! 刻意不复用 `lan_sync` 的 `GET /api/instances/:id`——那个会把整个实例目录打成 zip，
//! 在这里等于把「谁都能拉走你的整个游戏目录」。这里只认清单里登记过的**单个文件**。
//!
//! 分片靠 HTTP Range：服务端按请求切片读盘（不整文件载入内存），
//! 客户端按块追加到临时文件，断了可以从已有长度继续。

use std::collections::HashMap;
use std::io::{Read, SeekFrom};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Path as AxPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures_util::StreamExt;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use super::PackFile;

/// 一次最多返回多少字节（客户端也按这个量级分片）
const MAX_SLICE: u64 = 4 * 1024 * 1024;
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
const SLICE_TIMEOUT: Duration = Duration::from_secs(30);

/// 清单里的 kind → 实例目录下的子目录
pub fn kind_dir(kind: &str) -> Option<&'static str> {
    match kind {
        "mod" => Some("mods"),
        "resourcepack" => Some("resourcepacks"),
        "shaderpack" => Some("shaderpacks"),
        _ => None,
    }
}

#[derive(Clone)]
struct ServeState {
    base: PathBuf,
    token: String,
    /// key = "kind/name"，只有登记过的才服务
    files: Arc<HashMap<String, PackFile>>,
    fails: Arc<Mutex<HashMap<IpAddr, FailureWindow>>>,
    streams: Arc<tokio::sync::Semaphore>,
}

/// 定长比较，避免「按字节提前返回」泄露 token 前缀
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

const MAX_TOKEN_FAILS: usize = 20;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const MAX_FAILURE_PEERS: usize = 1024;

struct FailureWindow {
    since: Instant,
    count: usize,
}

fn record_failure(fails: &mut HashMap<IpAddr, FailureWindow>, peer: IpAddr, now: Instant) -> StatusCode {
    fails.retain(|_, window| now.duration_since(window.since) < FAILURE_WINDOW);
    if !fails.contains_key(&peer) && fails.len() >= MAX_FAILURE_PEERS {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let window = fails.entry(peer).or_insert(FailureWindow { since: now, count: 0 });
    if window.count >= MAX_TOKEN_FAILS {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    window.count += 1;
    StatusCode::FORBIDDEN
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, msg.to_string()).into_response()
}

/// 校验凭据：优先 `Authorization: Bearer`，兼容 `?token=`
///
/// 走 frp 中继时 URL 会落进对端 nginx 的 access log，
/// 所以中继模式下客户端一律用 header，query 只为兼容旧调用保留。
fn check_auth(
    st: &ServeState,
    peer: IpAddr,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Result<(), Response> {
    let from_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let provided = from_header.or_else(|| q.get("token").map(String::as_str));
    if provided.is_some_and(|token| ct_eq(token, &st.token)) {
        return Ok(());
    }
    let mut fails = st.fails.lock().unwrap_or_else(|error| error.into_inner());
    let status = record_failure(&mut fails, peer, Instant::now());
    Err(err(status, if status == StatusCode::FORBIDDEN { "bad token" } else { "too many attempts" }))
}

async fn handle_manifest(
    State(st): State<ServeState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_auth(&st, peer.ip(), &q, &headers) {
        return r;
    }
    let list: Vec<&PackFile> = st.files.values().collect();
    match serde_json::to_vec(&list) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "encode"),
    }
}

async fn handle_file(
    State(st): State<ServeState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    AxPath((kind, name)): AxPath<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if let Err(r) = check_auth(&st, peer.ip(), &q, &headers) {
        return r;
    }
    let key = format!("{kind}/{name}");
    let Some(entry) = st.files.get(&key) else {
        // 只服务清单里登记过的文件：这里是防任意文件读取的关键
        return err(StatusCode::NOT_FOUND, "not in manifest");
    };
    let Some(sub) = kind_dir(&kind) else {
        return err(StatusCode::NOT_FOUND, "bad kind");
    };
    // 清单里的 name 是纯文件名（生成时来自 read_dir），这里再挡一次路径穿越
    if !crate::fsutil::is_portable_filename(&name) {
        return err(StatusCode::BAD_REQUEST, "bad name");
    }
    let total = entry.size;
    if total > MAX_FILE {
        return err(StatusCode::PAYLOAD_TOO_LARGE, "file too large");
    }
    let value = headers.get(header::RANGE).map(|value| value.to_str().unwrap_or(""));
    let Some((start, end)) = bounded_range(value, total) else {
        return (StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{total}"))]).into_response();
    };
    let Ok(permit) = st.streams.clone().try_acquire_owned() else {
        return err(StatusCode::TOO_MANY_REQUESTS, "too many active transfers");
    };
    let relative = format!("{sub}/{name}");
    let Ok(path) = crate::fsutil::resolve_in_dir(&st.base, &relative, "房间文件") else {
        return err(StatusCode::NOT_FOUND, "not in instance");
    };
    let metadata = tokio::fs::symlink_metadata(st.base.join(&relative)).await;
    if !metadata.is_ok_and(|m| m.is_file() && !crate::fsutil::is_link_or_reparse_point(&m) && m.len() == total) {
        return err(StatusCode::CONFLICT, "file changed since manifest");
    }
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return err(StatusCode::NOT_FOUND, "file missing");
    };
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "read");
    }
    let length = end - start + 1;
    let stream = futures_util::stream::try_unfold((file, length, permit), |(mut file, remaining, permit)| async move {
        if remaining == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        let mut chunk = vec![0; remaining.min(64 * 1024) as usize];
        file.read_exact(&mut chunk).await?;
        let remaining = remaining - chunk.len() as u64;
        Ok(Some((bytes::Bytes::from(chunk), (file, remaining, permit))))
    });
    (
        StatusCode::PARTIAL_CONTENT,
        [
            (header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}")),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_string()),
        ],
        axum::body::Body::from_stream(stream),
    ).into_response()
}

fn bounded_range(value: Option<&str>, total: u64) -> Option<(u64, u64)> {
    let final_byte = total.checked_sub(1)?;
    let (start, requested_end) = match value {
        None => (0, final_byte),
        Some(value) => {
            let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
            let end = if end.is_empty() { final_byte } else { end.parse::<u64>().ok()? };
            (start.parse::<u64>().ok()?, end)
        }
    };
    let end = requested_end.min(final_byte).min(start.saturating_add(MAX_SLICE - 1));
    (start <= end).then_some((start, end))
}

/// 起本地 HTTP 服务（只绑 127.0.0.1），返回实际端口与后台任务
///
/// `port` 传 0 表示随机端口；走 frp 中继时必须用固定端口（隧道配置里写死的那个）。
pub async fn serve(
    base: PathBuf,
    files: Vec<PackFile>,
    token: String,
    port: u16,
) -> Result<(u16, tokio::task::JoinHandle<()>), String> {
    let map: HashMap<String, PackFile> = files
        .into_iter()
        .map(|f| (format!("{}/{}", f.kind, f.name), f))
        .collect();
    let st = ServeState {
        base,
        token,
        files: Arc::new(map),
        fails: Arc::new(Mutex::new(HashMap::new())),
        streams: Arc::new(tokio::sync::Semaphore::new(8)),
    };
    let app = Router::new()
        .route("/pack/manifest", get(handle_manifest))
        .route("/pack/file/:kind/:name", get(handle_file))
        .with_state(st);
    // 只绑回环：对外只能通过 frpc / EasyTier 转发进来，绝不直接暴露
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("监听本地端口 {port} 失败: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await;
    });
    Ok((port, handle))
}

/// 流式读取每个有界分片，校验整文件后提交。失败时保留完整的已接收分片。
pub async fn download_file(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    file: &PackFile,
    dest_dir: &Path,
    expected_sha1: &str,
) -> Result<u64, String> {
    let sub = kind_dir(&file.kind).ok_or_else(|| format!("未知类型 {}", file.kind))?;
    if !crate::fsutil::is_portable_filename(&file.name) || file.size > MAX_FILE {
        return Err("非法文件名或超过房间传输的单文件大小限制".into());
    }
    if expected_sha1.len() != 40 || !expected_sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("房间文件缺少有效 SHA-1 校验值".into());
    }
    let dir = crate::fsutil::resolve_in_dir(dest_dir, sub, "房间文件目录")?;
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("建目录失败: {e}"))?;
    let final_path = dir.join(&file.name);
    let part_path = dir.join(format!("{}.part", file.name));
    let mut written = match tokio::fs::symlink_metadata(&part_path).await {
        Ok(meta) if meta.is_file() && !crate::fsutil::is_link_or_reparse_point(&meta) => meta.len(),
        Ok(_) => return Err("房间文件的暂存路径不是普通文件".into()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
        Err(err) => return Err(format!("读取暂存文件失败: {err}")),
    };
    if written > file.size {
        tokio::fs::remove_file(&part_path).await.map_err(|e| e.to_string())?;
        written = 0;
    }
    let mut output = tokio::fs::OpenOptions::new().create(true).append(true).open(&part_path).await
        .map_err(|e| format!("打开临时文件失败: {e}"))?;
    let name = crate::modrinth::urlencode(&file.name);
    let url = format!("{origin}/pack/file/{}/{name}", file.kind);
    while written < file.size {
        let end = (written + MAX_SLICE - 1).min(file.size - 1);
        let response = client.get(&url)
            .timeout(SLICE_TIMEOUT)
            .bearer_auth(token)
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={written}-{end}"))
            .send().await.map_err(|e| format!("请求房主失败: {e}"))?;
        let length = validate_slice_headers(response.status(), response.headers(), written, end, file.size)?;
        let mut bytes = Vec::with_capacity(length);
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("读取分片失败: {e}"))?;
            if chunk.len() > length.saturating_sub(bytes.len()) {
                return Err("房主响应超过当前分片长度".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != length {
            return Err("房主响应与请求的分片长度不符".into());
        }
        output.write_all(&bytes).await.map_err(|e| format!("写临时文件失败: {e}"))?;
        written += length as u64;
    }
    output.sync_all().await.map_err(|e| format!("同步临时文件失败: {e}"))?;
    drop(output);
    let verify_path = part_path.clone();
    let (length, checksum) = tokio::task::spawn_blocking(move || {
        let length = std::fs::metadata(&verify_path).map_err(|e| e.to_string())?.len();
        Ok::<_, String>((length, sha1_of(&verify_path)?))
    }).await.map_err(|e| e.to_string())??;
    if length != file.size || !checksum.eq_ignore_ascii_case(expected_sha1) {
        tokio::fs::remove_file(&part_path).await.map_err(|e| format!("校验失败，无法清理暂存文件: {e}"))?;
        return Err(format!("{} 长度或校验失败", file.name));
    }
    tokio::fs::rename(&part_path, &final_path).await.map_err(|e| format!("落盘失败: {e}"))?;
    Ok(written)
}

fn validate_slice_headers(
    status: StatusCode,
    headers: &HeaderMap,
    start: u64,
    end: u64,
    total: u64,
) -> Result<usize, String> {
    if status != StatusCode::PARTIAL_CONTENT {
        return Err(format!("房主返回 HTTP {}，需要 206 分片响应", status.as_u16()));
    }
    let range = headers.get(header::CONTENT_RANGE).and_then(|header| header.to_str().ok())
        .and_then(crate::download::parse_content_range);
    if range != Some((start, end, total)) {
        return Err("房主返回的 Content-Range 与请求不符".into());
    }
    let length = (end - start + 1) as usize;
    if length as u64 > MAX_SLICE {
        return Err("分片超过大小限制".into());
    }
    if headers.get(header::CONTENT_LENGTH).is_some_and(|value|
        value.to_str().ok().and_then(|value| value.parse::<usize>().ok()) != Some(length)) {
        return Err("分片的 Content-Length 与请求不符".into());
    }
    if headers.get(header::CONTENT_ENCODING).is_some_and(|value| value != "identity") {
        return Err("房间分片必须使用原始字节编码".into());
    }
    Ok(length)
}

fn sha1_of(path: &Path) -> Result<String, String> {
    use sha1::{Digest, Sha1};
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qkx_pack_test_{tag}"));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// 本地回环跑通整条链路：起服务 → 分片下载 → sha1 校验 → 落盘。
    /// 不依赖 EasyTier，隧道那层单独验。
    #[test]
    fn serve_and_download_roundtrip() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("serve");
            let mods = root.join("mods");
            std::fs::create_dir_all(&mods).unwrap();
            // 跨越 4 MiB 边界，验证真实多分片下载。
            let payload: Vec<u8> = (0..5 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
            let src = mods.join("big.jar");
            std::fs::write(&src, &payload).unwrap();
            let sha = sha1_of(&src).unwrap();

            let file = PackFile {
                kind: "mod".into(),
                name: "big.jar".into(),
                mod_id: "big".into(),
                version: "1".into(),
                size: payload.len() as u64,
                sha1: sha.clone(),
                source: None,
                optional: false,
            };
            let (port, handle) = serve(root.clone(), vec![file.clone()], "tok".into(), 0)
                .await
                .unwrap();
            let client = reqwest::Client::new();
            let dest = temp_dir("dest");
            let n = download_file(
                &client,
                &format!("http://127.0.0.1:{port}"),
                "tok",
                &file,
                &dest,
                &sha,
            )
            .await
            .unwrap();
            assert_eq!(n, payload.len() as u64);
            let got = std::fs::read(dest.join("mods").join("big.jar")).unwrap();
            assert_eq!(got, payload);
            // 没有 .part 残留
            assert!(!dest.join("mods").join("big.jar.part").exists());
            handle.abort();
        });
    }

    /// 断点续传：已存在半截 .part 时从断点继续，最终内容正确
    #[test]
    fn resumes_from_partial_file() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("resume_src");
            std::fs::create_dir_all(root.join("mods")).unwrap();
            let payload: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 253) as u8).collect();
            std::fs::write(root.join("mods").join("r.jar"), &payload).unwrap();
            let sha = sha1_of(&root.join("mods").join("r.jar")).unwrap();
            let file = PackFile {
                kind: "mod".into(),
                name: "r.jar".into(),
                mod_id: "r".into(),
                version: "1".into(),
                size: payload.len() as u64,
                sha1: sha.clone(),
                source: None,
                optional: false,
            };
            let (port, handle) = serve(root, vec![file.clone()], "tok".into(), 0).await.unwrap();
            let dest = temp_dir("resume_dst");
            std::fs::create_dir_all(dest.join("mods")).unwrap();
            // 先放好前 512KB，模拟上次断在这里
            std::fs::write(dest.join("mods").join("r.jar.part"), &payload[..512 * 1024]).unwrap();

            let client = reqwest::Client::new();
            let n = download_file(
                &client,
                &format!("http://127.0.0.1:{port}"),
                "tok",
                &file,
                &dest,
                &sha,
            )
            .await
            .unwrap();
            assert_eq!(n, payload.len() as u64);
            let got = std::fs::read(dest.join("mods").join("r.jar")).unwrap();
            assert_eq!(got, payload, "续传后的内容必须与原文件一致");
            handle.abort();
        });
    }

    /// 校验失败必须删掉 .part，不能把坏文件留在实例目录里
    #[test]
    fn bad_checksum_leaves_no_file() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("bad_src");
            std::fs::create_dir_all(root.join("mods")).unwrap();
            std::fs::write(root.join("mods").join("b.jar"), b"hello world").unwrap();
            let file = PackFile {
                kind: "mod".into(),
                name: "b.jar".into(),
                mod_id: "b".into(),
                version: "1".into(),
                size: 11,
                sha1: "wrong".into(),
                source: None,
                optional: false,
            };
            let (port, handle) = serve(root, vec![file.clone()], "tok".into(), 0).await.unwrap();
            let dest = temp_dir("bad_dst");
            let client = reqwest::Client::new();
            let err = download_file(
                &client,
                &format!("http://127.0.0.1:{port}"),
                "tok",
                &file,
                &dest,
                &"0".repeat(40),
            )
            .await
            .unwrap_err();
            assert!(err.contains("校验失败"), "错误信息应指出校验失败: {err}");
            assert!(!dest.join("mods").join("b.jar").exists(), "不应生成正式文件");
            assert!(
                !dest.join("mods").join("b.jar.part").exists(),
                "坏数据不应留在 .part"
            );
            handle.abort();
        });
    }

    /// 限流只拦截错误凭据。共享同一个转发地址的合法成员仍可下载。
    #[test]
    fn limits_invalid_tokens_without_blocking_valid_members() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("lockout_src");
            let (port, handle) = serve(root, vec![], "secret-token".into(), 0).await.unwrap();
            let client = reqwest::Client::new();
            let base = format!("http://127.0.0.1:{port}/pack/manifest");

            for i in 0..MAX_TOKEN_FAILS {
                let r = client
                    .get(&base)
                    .query(&[("token", format!("wrong{i}"))])
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), reqwest::StatusCode::FORBIDDEN);
            }
            let limited = client.get(&base).bearer_auth("wrong").send().await.unwrap();
            assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
            let r = client
                .get(&base)
                .bearer_auth("secret-token")
                .send()
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            handle.abort();
        });
    }

    /// 凭据可以用 Authorization 头传（中继模式下不再把 token 写进 URL）
    #[test]
    fn accepts_bearer_header() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("bearer_src");
            let (port, handle) = serve(root, vec![], "tok2".into(), 0).await.unwrap();
            let client = reqwest::Client::new();
            let ok = client
                .get(format!("http://127.0.0.1:{port}/pack/manifest"))
                .bearer_auth("tok2")
                .send()
                .await
                .unwrap();
            assert_eq!(ok.status(), reqwest::StatusCode::OK);
            let bad = client
                .get(format!("http://127.0.0.1:{port}/pack/manifest"))
                .bearer_auth("nope")
                .send()
                .await
                .unwrap();
            assert_eq!(bad.status(), reqwest::StatusCode::FORBIDDEN);
            handle.abort();
        });
    }

    #[test]
    fn unknown_file_and_bad_token_are_rejected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("reject");
            std::fs::create_dir_all(root.join("mods")).unwrap();
            let (port, handle) = serve(root, vec![], "tok".into(), 0).await.unwrap();
            let c = reqwest::Client::new();
            // 清单里没有的文件 → 404（防任意文件读取）
            let r = c
                .get(format!("http://127.0.0.1:{port}/pack/file/mod/secret.txt?token=tok"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 404);
            // token 不对 → 403
            let r = c
                .get(format!("http://127.0.0.1:{port}/pack/manifest?token=wrong"))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 403);
            handle.abort();
        });
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;

    #[test]
    fn an_explicit_range_is_capped_and_response_range_must_match() {
        let total = MAX_SLICE * 3;
        assert_eq!(bounded_range(Some("bytes=1-999999999"), total), Some((1, MAX_SLICE)));
        assert_eq!(bounded_range(Some("bytes=1-20,50-70"), total), None);
        assert_eq!(bounded_range(Some("bytes=18446744073709551615-"), total), None);
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 2-5/10".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "4".parse().unwrap());
        assert_eq!(validate_slice_headers(StatusCode::PARTIAL_CONTENT, &headers, 2, 5, 10).unwrap(), 4);
        assert!(validate_slice_headers(StatusCode::OK, &headers, 2, 5, 10).is_err());
        assert!(validate_slice_headers(StatusCode::PARTIAL_CONTENT, &headers, 1, 5, 10).is_err());
        headers.insert(header::CONTENT_LENGTH, "12".parse().unwrap());
        assert!(validate_slice_headers(StatusCode::PARTIAL_CONTENT, &headers, 2, 5, 10).is_err());
    }

    #[test]
    fn repeated_invalid_tokens_expire_per_peer_and_do_not_block_a_valid_token() {
        let st = ServeState {
            base: PathBuf::new(),
            token: "fixture-token".into(),
            files: Arc::new(HashMap::new()),
            fails: Arc::new(Mutex::new(HashMap::new())),
            streams: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        let now = Instant::now();
        let one = "127.0.0.1".parse().unwrap();
        let another = "127.0.0.2".parse().unwrap();
        {
            let mut failures = st.fails.lock().unwrap();
            for _ in 0..MAX_TOKEN_FAILS {
                assert_eq!(record_failure(&mut failures, one, now), StatusCode::FORBIDDEN);
            }
            assert_eq!(record_failure(&mut failures, one, now), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(record_failure(&mut failures, another, now), StatusCode::FORBIDDEN);
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer fixture-token".parse().unwrap());
        assert!(check_auth(&st, one, &HashMap::new(), &headers).is_ok());
        let mut failures = st.fails.lock().unwrap();
        assert_eq!(record_failure(&mut failures, one, now + FAILURE_WINDOW), StatusCode::FORBIDDEN);
    }
}
