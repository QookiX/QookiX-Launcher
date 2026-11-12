//! 单文件 HTTP 服务（房主侧）与分片下载器（客人侧）。
//!
//! 刻意不复用 `lan_sync` 的 `GET /api/instances/:id`——那个会把整个实例目录打成 zip，
//! 在这里等于把「谁都能拉走你的整个游戏目录」。这里只认清单里登记过的**单个文件**。
//!
//! 分片靠 HTTP Range：服务端按请求切片读盘（不整文件载入内存），
//! 客户端按块追加到临时文件，断了可以从已有长度继续。

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::{Path as AxPath, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use super::PackFile;

/// 一次最多返回多少字节（客户端也按这个量级分片）
const MAX_SLICE: u64 = 4 * 1024 * 1024;

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
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, msg.to_string()).into_response()
}

async fn handle_manifest(
    State(st): State<ServeState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if q.get("token").map(String::as_str) != Some(st.token.as_str()) {
        return err(StatusCode::FORBIDDEN, "bad token");
    }
    let list: Vec<&PackFile> = st.files.values().collect();
    match serde_json::to_vec(&list) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "encode"),
    }
}

async fn handle_file(
    State(st): State<ServeState>,
    AxPath((kind, name)): AxPath<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if q.get("token").map(String::as_str) != Some(st.token.as_str()) {
        return err(StatusCode::FORBIDDEN, "bad token");
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
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return err(StatusCode::BAD_REQUEST, "bad name");
    }
    let path = st.base.join(sub).join(&name);

    let total = entry.size;
    if total == 0 {
        return (StatusCode::OK, Vec::<u8>::new()).into_response();
    }
    // 解析 Range: bytes=a-b；没写就从头给一片
    let (mut start, mut end) = (0u64, total.saturating_sub(1).min(MAX_SLICE - 1));
    if let Some(raw) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        if let Some(rest) = raw.strip_prefix("bytes=") {
            let spec = rest.split(',').next().unwrap_or("");
            let mut parts = spec.splitn(2, '-');
            let a = parts.next().unwrap_or("").trim();
            let b = parts.next().unwrap_or("").trim();
            if let Ok(v) = a.parse::<u64>() {
                start = v;
            }
            if let Ok(v) = b.parse::<u64>() {
                end = v.min(total.saturating_sub(1));
            } else {
                end = (start + MAX_SLICE - 1).min(total.saturating_sub(1));
            }
        }
    }
    if start > end || start >= total {
        return err(StatusCode::RANGE_NOT_SATISFIABLE, "bad range");
    }
    let len = (end - start + 1) as usize;

    let mut buf = vec![0u8; len];
    let read = (|| -> std::io::Result<usize> {
        let mut f = std::fs::File::open(&path)?;
        f.seek(SeekFrom::Start(start))?;
        f.read(&mut buf)
    })();
    let n = match read {
        Ok(n) => n,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "read"),
    };
    buf.truncate(n);

    let mut resp = (StatusCode::PARTIAL_CONTENT, buf).into_response();
    let h = resp.headers_mut();
    if let Ok(v) = format!("bytes {start}-{}/{total}", start + n as u64 - 1).parse() {
        h.insert(header::CONTENT_RANGE, v);
    }
    h.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    resp
}

/// 起本地 HTTP 服务（只绑 127.0.0.1），返回实际端口与后台任务
pub async fn serve(
    base: PathBuf,
    files: Vec<PackFile>,
    token: String,
) -> Result<(u16, tokio::task::JoinHandle<()>), String> {
    let map: HashMap<String, PackFile> = files
        .into_iter()
        .map(|f| (format!("{}/{}", f.kind, f.name), f))
        .collect();
    let st = ServeState {
        base,
        token,
        files: Arc::new(map),
    };
    let app = Router::new()
        .route("/pack/manifest", get(handle_manifest))
        .route("/pack/file/:kind/:name", get(handle_file))
        .with_state(st);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("监听本地端口失败: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((port, handle))
}

/// 下载单个文件到 `dest`（先写 `.part`，校验通过再改名）
///
/// `expected_sha1` 为空时跳过校验（调用方自己兜底）。
pub async fn download_file(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    file: &PackFile,
    dest_dir: &Path,
    expected_sha1: &str,
) -> Result<u64, String> {
    let Some(sub) = kind_dir(&file.kind) else {
        return Err(format!("未知类型 {}", file.kind));
    };
    if file.name.contains('/') || file.name.contains('\\') || file.name.contains("..") {
        return Err("非法文件名".into());
    }
    let dir = dest_dir.join(sub);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败: {e}"))?;
    let final_path = dir.join(&file.name);
    let part_path = dir.join(format!("{}.part", file.name));

    let mut written: u64 = std::fs::metadata(&part_path).map(|m| m.len()).unwrap_or(0);
    if written > file.size {
        // 长度对不上说明是上一次残留的坏数据，重来
        let _ = std::fs::remove_file(&part_path);
        written = 0;
    }

    while written < file.size {
        let end = (written + MAX_SLICE - 1).min(file.size - 1);
        let url = format!("{origin}/pack/file/{}/{}?token={}", file.kind, file.name, token);
        let resp = client
            .get(&url)
            .header(header::RANGE, format!("bytes={written}-{end}"))
            .send()
            .await
            .map_err(|e| format!("请求房主失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("房主返回 HTTP {}", resp.status().as_u16()));
        }
        let bytes = resp.bytes().await.map_err(|e| format!("读取数据失败: {e}"))?;
        if bytes.is_empty() {
            return Err("房主返回空数据".into());
        }
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&part_path)
                .map_err(|e| format!("写临时文件失败: {e}"))?;
            f.write_all(&bytes).map_err(|e| format!("写入失败: {e}"))?;
        }
        written += bytes.len() as u64;
    }

    // 整文件校验：只认清单里的 sha1，对不上就删掉，绝不留半成品进实例目录
    if !expected_sha1.is_empty() {
        let actual = sha1_of(&part_path)?;
        if actual != expected_sha1 {
            let _ = std::fs::remove_file(&part_path);
            return Err(format!("{} 校验失败（期望 {expected_sha1}）", file.name));
        }
    }
    std::fs::rename(&part_path, &final_path).map_err(|e| format!("落盘失败: {e}"))?;
    Ok(written)
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
            // 3 MB 的文件，强制走多个 4MB 以内的分片
            let payload: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
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
            let (port, handle) = serve(root.clone(), vec![file.clone()], "tok".into())
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
            let (port, handle) = serve(root, vec![file.clone()], "tok".into()).await.unwrap();
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
            let (port, handle) = serve(root, vec![file.clone()], "tok".into()).await.unwrap();
            let dest = temp_dir("bad_dst");
            let client = reqwest::Client::new();
            let err = download_file(
                &client,
                &format!("http://127.0.0.1:{port}"),
                "tok",
                &file,
                &dest,
                "0000deadbeef",
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

    #[test]
    fn unknown_file_and_bad_token_are_rejected() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let root = temp_dir("reject");
            std::fs::create_dir_all(root.join("mods")).unwrap();
            let (port, handle) = serve(root, vec![], "tok".into()).await.unwrap();
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
