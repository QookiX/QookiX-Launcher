use crate::models::{ServerConfig, ServerCore};
use crate::state::AppState;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::Emitter;



/// 服务器 ID 只允许单层名称，且限定 ASCII 字母数字与 `-_.`。
pub fn validate_server_id(id: &str) -> Result<(), String> {
    crate::fsutil::validate_id(id, "服务器")?;
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("非法服务器 ID".into());
    }
    Ok(())
}

pub fn server_dir(state: &AppState, id: &str) -> PathBuf {
    state.servers_dir().join(id)
}

pub fn server_meta_path(state: &AppState, id: &str) -> PathBuf {
    server_dir(state, id).join("server.json")
}

pub fn load_servers(state: &AppState) -> Vec<ServerConfig> {
    crate::fsutil::load_dir_entries(&state.servers_dir(), "server.json", |s: &ServerConfig| s.created)
}

pub fn get_server(state: &AppState, id: &str) -> Result<ServerConfig, String> {
    validate_server_id(id)?;
    let text = std::fs::read_to_string(server_meta_path(state, id))
        .map_err(|_| format!("服务器 {id} 不存在"))?;
    serde_json::from_str(&text).map_err(|e| format!("服务器数据损坏: {e}"))
}

pub fn save_server(state: &AppState, server: &ServerConfig) -> Result<(), String> {
    let path = server_meta_path(state, &server.id);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(server).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// 从服务器的 `server.properties` 读取 `motd`，并解码 Java properties 转义
/// （至少把 `\u00A7` 还原成 `§`，`\n` 还原成换行），让前端能按 MC 颜色码渲染。
/// 文件不存在或没有 motd 行时返回 `None`，由调用方回退到 server.json 里的旧值。
pub fn read_server_motd(state: &AppState, id: &str) -> Option<String> {
    let path = server_dir(state, id).join("server.properties");
    let text = std::fs::read_to_string(&path).ok()?;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let eq = match t.find('=') {
            Some(i) => i,
            None => continue,
        };
        if t[..eq].trim() == "motd" {
            return Some(decode_java_properties(t[eq + 1..].trim()));
        }
    }
    None
}

/// 解码 Java `.properties` 的转义序列：`\uXXXX`、`\n`、`\t`、`\r`、`\\`、`\"`、`\'`。
/// 无效的 `\u` 原样保留反斜杠。
fn decode_java_properties(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'u' if i + 6 <= chars.len() => {
                    let hex: String = chars[i + 2..i + 6].iter().collect();
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                            i += 6;
                            continue;
                        }
                    }
                    out.push('\\');
                    i += 1;
                    continue;
                }
                'n' => {
                    out.push('\n');
                    i += 2;
                    continue;
                }
                't' => {
                    out.push('\t');
                    i += 2;
                    continue;
                }
                'r' => {
                    out.push('\r');
                    i += 2;
                    continue;
                }
                '\\' => {
                    out.push('\\');
                    i += 2;
                    continue;
                }
                '"' => {
                    out.push('"');
                    i += 2;
                    continue;
                }
                '\'' => {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn create_server(
    state: &AppState,
    name: String,
    core: ServerCore,
    mc_version: String,
) -> Result<ServerConfig, String> {
    let name = name.trim().to_string();
    let name = if name.is_empty() {
        let mut auto = core.as_str().to_string();
        auto[0..1].make_ascii_uppercase();
        auto.push_str(" Server");
        auto
    } else {
        name
    };
    if mc_version.is_empty() {
        return Err("请选择 Minecraft 版本".into());
    }
    let mut id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    while server_meta_path(state, &id).exists() {
        id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    }
    let server = ServerConfig {
        id,
        name,
        core,
        mc_version,
        port: 25565,
        max_memory_mb: 2048,
        min_memory_mb: 1024,
        motd: "A Minecraft Server".into(),
        eula: false,
        created: crate::util::now_secs(),
        last_started: None,
        java_path: None,
        jvm_args: None,
        stop_command: None,
    };
    std::fs::create_dir_all(server_dir(state, &server.id)).map_err(|e| e.to_string())?;
    save_server(state, &server)?;
    Ok(server)
}

pub fn update_server(state: &AppState, patch: serde_json::Value) -> Result<ServerConfig, String> {
    let id = patch
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("缺少服务器 id")?;
    let mut s = get_server(state, id)?;
    if let Some(v) = patch.get("name").and_then(|v| v.as_str()) {
        let n = v.trim().to_string();
        if !n.is_empty() {
            s.name = n;
        }
    }
    if let Some(v) = patch.get("port").and_then(|v| v.as_u64()) {
        s.port = (v as u32).min(65535).max(1) as u16;
    }
    if let Some(v) = patch.get("max_memory_mb").and_then(|v| v.as_u64()) {
        s.max_memory_mb = v as u32;
    }
    if let Some(v) = patch.get("min_memory_mb").and_then(|v| v.as_u64()) {
        s.min_memory_mb = v as u32;
    }
    if let Some(v) = patch.get("motd").and_then(|v| v.as_str()) {
        s.motd = v.to_string();
    }
    if let Some(v) = patch.get("eula").and_then(|v| v.as_bool()) {
        s.eula = v;
    }
    if let Some(v) = patch.get("java_path").and_then(|v| v.as_str()) {
        s.java_path = if v.is_empty() { None } else { Some(v.to_string()) };
    }
    if let Some(v) = patch.get("jvm_args").and_then(|v| v.as_str()) {
        s.jvm_args = if v.is_empty() { None } else { Some(v.to_string()) };
    }
    if let Some(v) = patch.get("stop_command").and_then(|v| v.as_str()) {
        s.stop_command = if v.is_empty() { None } else { Some(v.to_string()) };
    }
    save_server(state, &s)?;
    Ok(s)
}

pub fn delete_server(state: &AppState, id: &str) -> Result<(), String> {
    validate_server_id(id)?;
    let _files = state.jobs.reserve_server_files(id)?;
    let dir = server_dir(state, id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("删除服务器目录失败: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core jar installation
// ---------------------------------------------------------------------------

/// Download the server core jar into the server directory.
/// Download the server core into the server directory.
/// Vanilla: Mojang version manifest. Paper/Spigot/Purpur: PaperMC API.
/// Fabric/Forge: download + run installer headless.
pub async fn install_server_core(
    app: tauri::AppHandle,
    state: &AppState,
    id: &str,
) -> Result<(), String> {
    validate_server_id(id)?;
    let _files = state.jobs.reserve_server_files(id)?;
    let server = get_server(state, id)?;
    let dir = server_dir(state, id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let jar_path = dir.join("server.jar");

    match server.core {
        ServerCore::Vanilla => {
            let _ = app.emit(
                "server://install-progress",
                serde_json::json!({ "serverId": id, "phase": "正在获取版本信息…", "done": 0, "total": 1 }),
            );
            let vj = crate::mcmeta::fetch_version_json(state, &server.mc_version).await?;
            let dl = vj
                .downloads
                .server
                .ok_or_else(|| format!("版本 {} 没有官方服务端核心", server.mc_version))?;
            let dl_phase = format!("正在下载服务端核心（{}）…", server.mc_version);
            download_file_to_with_progress(&app, &state.client, &dl.url, &jar_path, id, &dl_phase, None).await?;
            let _ = app.emit(
                "server://install-progress",
                serde_json::json!({ "serverId": id, "phase": "完成", "done": 1, "total": 1 }),
            );
            Ok(())
        }
        ServerCore::Paper | ServerCore::Spigot => {
            download_papermc_core(&app, state, id, server.core, &server.mc_version).await
        }
        ServerCore::Purpur => {
            download_purpur_core(&app, state, id, &server.mc_version).await
        }
        ServerCore::Fabric => {
            download_fabric_core(&app, state, id, &server.mc_version).await
        }
        ServerCore::Forge => {
            download_forge_core(&app, state, id, &server.mc_version).await
        }
    }
}

/// 流式下载文件并实时 emit 下载进度（done/total 字节数）
async fn download_file_to_with_progress(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    id: &str,
    phase: &str,
    ua: Option<&str>,
) -> Result<(), String> {
    let mut req = client.get(url).timeout(std::time::Duration::from_secs(600));
    if let Some(u) = ua {
        req = req.header("User-Agent", u);
    }
    let resp = req.send().await.map_err(|e| format!("下载失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("下载失败: HTTP {status}"));
    }
    let total = resp.content_length().unwrap_or(0);
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }

    let result = download_stream_inner(app, resp, dest, id, phase, total).await;
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

async fn download_stream_inner(
    app: &tauri::AppHandle,
    resp: reqwest::Response,
    dest: &Path,
    id: &str,
    phase: &str,
    total: u64,
) -> Result<(), String> {
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("创建文件失败: {e}"))?;
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut stream = resp.bytes_stream();
    let mut done: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": phase, "done": 0, "total": total
    }));
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取数据失败: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入文件失败: {e}"))?;
        done += chunk.len() as u64;
        if last_emit.elapsed() >= std::time::Duration::from_millis(120) {
            let _ = app.emit("server://install-progress", serde_json::json!({
                "serverId": id, "phase": phase, "done": done, "total": total
            }));
            last_emit = std::time::Instant::now();
        }
    }
    file.flush().await.map_err(|e| format!("刷新文件失败: {e}"))?;
    if total > 0 && done != total {
        return Err(format!("下载不完整: 已下载 {done} 字节, 预期 {total} 字节"));
    }
    let final_total = if total > 0 { total } else { done };
    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": phase, "done": final_total, "total": final_total
    }));
    Ok(())
}

// ---------------------------------------------------------------------------
// Server core download — PaperMC (Paper/Spigot/Purpur)
// ---------------------------------------------------------------------------

const PAPERMC_API: &str = "https://fill.papermc.io/v3/projects";
const PAPERMC_UA: &str = "QookiX-Launcher/0.3.8 (https://github.com/weimosheng/QookiX-Launcher)";

fn papermc_project(core: ServerCore) -> &'static str {
    match core {
        ServerCore::Paper => "paper",
        ServerCore::Spigot => "paper", // Spigot 无预编译包，用 Paper（完全兼容 Spigot API）
        _ => "paper",
    }
}

async fn download_papermc_core(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    core: ServerCore,
    mc_version: &str,
) -> Result<(), String> {
    let project = papermc_project(core);
    let jar_path = server_dir(state, id).join("server.jar");
    let is_spigot = core == ServerCore::Spigot;
    let core_name = if is_spigot { "Paper (兼容 Spigot)" } else { core.as_str() };

    let init_phase = if is_spigot {
        "Spigot 无预编译包，使用兼容的 Paper 核心…"
    } else {
        "正在获取版本信息…"
    };
    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": init_phase, "done": 0, "total": 1
    }));

    let url = format!("{}/{}/versions/{}/builds", PAPERMC_API, project, mc_version);
    let resp = state
        .client
        .get(&url)
        .header("User-Agent", PAPERMC_UA)
        .send()
        .await
        .map_err(|e| format!("获取 {core_name} 版本信息失败: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("获取 {core_name} 版本信息失败: HTTP {status}"));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;

    if body.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(format!("{core_name}: {msg}"));
    }

    let builds = body.as_array().ok_or("返回数据格式异常")?;
    if builds.is_empty() {
        return Err(format!("{core_name} 暂无 {mc_version} 版本的构建"));
    }

    // 优先选 STABLE 构建，否则取最后一个
    let build = builds
        .iter()
        .rev()
        .find(|b| b.get("channel").and_then(|v| v.as_str()) == Some("STABLE"))
        .or_else(|| builds.last())
        .unwrap();

    let dl_url = build
        .get("downloads")
        .and_then(|d| d.get("server:default"))
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
        .ok_or("无法解析下载 URL")?;

    let dl_phase = format!("正在下载 {core_name} 核心…");
    download_file_to_with_progress(
        &app, &state.client, dl_url, &jar_path, id, &dl_phase, Some(PAPERMC_UA),
    )
    .await?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "完成", "done": 1, "total": 1
    }));
    Ok(())
}

/// Purpur 有独立的 API（api.purpurmc.org），不走 PaperMC
async fn download_purpur_core(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    mc_version: &str,
) -> Result<(), String> {
    let jar_path = server_dir(state, id).join("server.jar");

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在获取版本信息…", "done": 0, "total": 1
    }));
    let url = format!("https://api.purpurmc.org/v2/purpur/{}", mc_version);
    let resp: serde_json::Value = crate::download::get_json(&state.client, &url)
        .await
        .map_err(|e| format!("获取 Purpur 版本信息失败: {e}"))?;
    let latest = resp
        .get("builds")
        .and_then(|b| b.get("latest"))
        .and_then(|l| l.as_str())
        .ok_or("无法解析 Purpur 最新构建号")?;

    let dl_url = format!(
        "https://api.purpurmc.org/v2/purpur/{}/{}/download",
        mc_version, latest
    );
    download_file_to_with_progress(&app, &state.client, &dl_url, &jar_path, id, "正在下载 Purpur 核心…", None).await?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "完成", "done": 1, "total": 1
    }));
    Ok(())
}

// ---------------------------------------------------------------------------
// Server core download — installer-based (Fabric/Forge)
// ---------------------------------------------------------------------------

/// 从 maven-metadata.xml 中提取所有版本号
fn parse_maven_versions(xml: &str) -> Vec<String> {
    let mut versions = Vec::new();
    let mut in_versions = false;
    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<version>") && trimmed.ends_with("</version>") {
            if in_versions {
                let v = &trimmed[9..trimmed.len() - 10];
                versions.push(v.to_string());
            }
        } else if trimmed.starts_with("<versions>") {
            in_versions = true;
        } else if trimmed.starts_with("</versions>") {
            in_versions = false;
        }
    }
    versions
}

/// 运行 installer jar 并等待完成
async fn run_installer(
    java: &str,
    installer: &Path,
    dir: &Path,
    args: &[&str],
) -> Result<(), String> {
    let mut cmd = Command::new(java);
    cmd.arg("-jar")
        .arg(installer)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let output = tokio::process::Command::from(cmd)
        .output()
        .await
        .map_err(|e| format!("运行安装器失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("安装器执行失败: {}", stderr.trim()));
    }
    Ok(())
}

async fn download_fabric_core(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    mc_version: &str,
) -> Result<(), String> {
    let dir = server_dir(state, id);
    let java = crate::launch::find_best_java(state, 8)
        .await
        .ok_or("未找到 Java 运行时，请先在设置中配置 Java")?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在获取版本信息…", "done": 0, "total": 1
    }));

    let meta_url = crate::mirror::rewrite(
        state,
        &format!("https://meta.fabricmc.net/v2/versions/loader/{}", mc_version),
    );
    let maven_base = crate::mirror::rewrite(
        state,
        "https://maven.fabricmc.net/net/fabricmc/fabric-installer",
    );

    // 获取最新 loader 版本
    let resp: serde_json::Value = crate::download::get_json(&state.client, &meta_url)
        .await
        .map_err(|e| format!("获取 loader 版本失败: {e}"))?;
    let loader_ver = resp
        .get(0)
        .and_then(|v| v.get("loader"))
        .and_then(|l| l.get("version"))
        .and_then(|v| v.as_str())
        .or_else(|| resp.get(0).and_then(|v| v.get("version")).and_then(|v| v.as_str()))
        .ok_or("无法解析 loader 版本")?;

    // 获取最新 installer 版本
    let installer_meta = crate::mirror::rewrite(state, "https://meta.fabricmc.net/v2/versions/installer");
    let ir: serde_json::Value =
        crate::download::get_json(&state.client, &installer_meta)
            .await
            .map_err(|e| format!("获取 installer 版本失败: {e}"))?;
    let installer_ver = ir
        .get(0)
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .ok_or("无法解析 installer 版本")?;

    let installer_jar = format!("fabric-installer-{}.jar", installer_ver);
    let installer_url = format!("{}/{}/{}", maven_base, installer_ver, installer_jar);
    let installer_path = dir.join("installer.jar");

    download_file_to_with_progress(&app, &state.client, &installer_url, &installer_path, id, "正在下载安装器…", None).await?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在安装 Fabric 核心…", "done": 0, "total": 1
    }));

    let dir_str = dir.to_string_lossy().to_string();
    run_installer(
        &java.path,
        &installer_path,
        &dir,
        &["server", "-dir", &dir_str, "-mcversion", mc_version, "-loader", loader_ver],
    )
    .await?;

    let _ = std::fs::remove_file(&installer_path);

    // Fabric installer 不会自动下载原版 server jar，需手动下载（始终覆盖，避免旧 launcher 拋留）
    let vanilla_jar = dir.join("server.jar");
    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在下载原版服务端核心…", "done": 0, "total": 1
    }));
    let vj = crate::mcmeta::fetch_version_json(state, mc_version).await?;
    let dl = vj
        .downloads
        .server
        .ok_or_else(|| format!("版本 {} 没有官方服务端核心", mc_version))?;
    let vanilla_url = crate::mirror::server_jar_url(state, &dl.url, mc_version);
    download_file_to_with_progress(
        &app, &state.client, &vanilla_url, &vanilla_jar, id, "正在下载原版服务端核心…", None,
    )
    .await?;

    // 验证 launcher jar 已生成（不重命名，保留原版 server.jar）
    let mut found = false;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("fabric-server-") && name.ends_with(".jar") {
                found = true;
                break;
            }
        }
    }
    if !found {
        return Err("Fabric 安装完成但未找到服务端 launcher jar".into());
    }

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "完成", "done": 1, "total": 1
    }));
    Ok(())
}

async fn download_forge_core(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    mc_version: &str,
) -> Result<(), String> {
    let dir = server_dir(state, id);
    let java = crate::launch::find_best_java(state, 8)
        .await
        .ok_or("未找到 Java 运行时，请先在设置中配置 Java")?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在获取版本信息…", "done": 0, "total": 1
    }));

    let meta_url = crate::mirror::rewrite(
        state,
        "https://files.minecraftforge.net/maven/net/minecraftforge/forge/maven-metadata.xml",
    );
    let maven_base = crate::mirror::rewrite(
        state,
        "https://files.minecraftforge.net/maven/net/minecraftforge/forge",
    );

    let xml = crate::download::get_text(&state.client, &meta_url)
        .await
        .map_err(|e| format!("获取版本元数据失败: {e}"))?;
    let versions = parse_maven_versions(&xml);

    // 找匹配 mc_version 的最新版本（格式: {mc}-{build}）
    let prefix = format!("{}-", mc_version);
    let version = versions
        .iter()
        .rev()
        .find(|v| v.starts_with(&prefix))
        .ok_or(format!("未找到 Forge {mc_version} 的版本"))?;

    let installer_url = format!("{}/{}/forge-{}-installer.jar", maven_base, version, version);
    let installer_path = dir.join("installer.jar");

    download_file_to_with_progress(&app, &state.client, &installer_url, &installer_path, id, "正在下载安装器…", None).await?;

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "正在安装 Forge 核心（可能需要数分钟）…", "done": 0, "total": 1
    }));
    run_installer(&java.path, &installer_path, &dir, &["--installServer"]).await?;

    let _ = std::fs::remove_file(&installer_path);

    let _ = app.emit("server://install-progress", serde_json::json!({
        "serverId": id, "phase": "完成", "done": 1, "total": 1
    }));
    Ok(())
}

// ---------------------------------------------------------------------------
// Start / stop
// ---------------------------------------------------------------------------

/// 在 libraries/ 目录下递归查找 Forge 的 args 文件
fn find_args_file(dir: &Path) -> Option<PathBuf> {
    let args_name = if cfg!(windows) { "win_args.txt" } else { "unix_args.txt" };
    fn search(dir: &Path, name: &str) -> Option<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else { return None };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Some(r) = search(&p, name) {
                    return Some(r);
                }
            } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(p);
            }
        }
        None
    }
    search(&dir.join("libraries"), args_name)
}

/// 查找 Fabric 的 server launcher jar（如 fabric-server-*.jar）
fn find_launcher_jar(dir: &Path, prefix: &str) -> Option<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return None };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(prefix) && name.ends_with(".jar") {
            return Some(name);
        }
    }
    None
}

fn ensure_server_files(dir: &Path, eula: bool) -> Result<(), String> {
    if eula {
        std::fs::write(dir.join("eula.txt"), "eula=true\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn pick_java(state: &AppState, jar: &Path, mc_version: &str, java_override: Option<&str>) -> Result<(crate::models::JavaInfo, u32), String> {
    // 先计算所需 Java 版本
    let mut required = 8u32;
    // 优先从 Mojang 版本清单获取官方指定的 Java 版本
    if let Ok(vj) = crate::mcmeta::fetch_version_json(state, mc_version).await {
        if let Some(jv) = vj.java_version {
            if jv.major_version > required {
                required = jv.major_version;
            }
        }
    }
    // 启发式备选：根据 MC 版本推断 Java 需求
    let estimated = estimate_java_version(mc_version);
    if estimated > required {
        required = estimated;
    }
    // jar 字节码版本作为补充
    if let Some(class_major) = crate::archive::jar_class_version(jar) {
        let from_jar = class_major.saturating_sub(44);
        if from_jar > required {
            required = from_jar;
        }
    }

    // 检查用户配置的 Java 路径
    let configured = java_override
        .filter(|p| !p.is_empty() && Path::new(p).exists())
        .map(|p| p.to_string())
        .or_else(|| {
            let s = state.settings();
            s.java_path.clone()
        });
    if let Some(p) = configured {
        if !p.is_empty() && Path::new(&p).exists() {
            if let Some(info) = crate::java::probe_java(Path::new(&p)) {
                if info.major >= required || required == 0 {
                    return Ok((info, required));
                }
                // 配置的 Java 版本不足，尝试自动选更高的
                if let Some(j) = crate::launch::find_best_java(state, required).await {
                    if j.major >= required {
                        return Ok((j, required));
                    }
                }
                return Err(format!(
                    "配置的 Java {} 不满足要求（需要 Java {required}+），请在设置中更换 Java 路径或安装 JDK {required}",
                    info.major
                ));
            }
            return Ok((crate::models::JavaInfo {
                path: p,
                version: "?".into(),
                major: 0,
                vendor: "?".into(),
                arch: "?".into(),
            }, required));
        }
    }
    if let Some(j) = crate::launch::find_best_java(state, required).await {
        if j.major > 0 && j.major < required {
            return Err(format!(
                "服务器核心需要 Java {required}+，但系统最高仅 Java {}，请安装 JDK {required} 或更高版本",
                j.major
            ));
        }
        return Ok((j, required));
    }
    Err(format!(
        "未找到合适的 Java 运行时，该服务器核心需要 Java {required}+，请在设置中配置 Java 路径或安装 JDK {required}"
    ))
}

/// 根据 MC 版本号启发式推断所需 Java 大版本
fn estimate_java_version(mc_version: &str) -> u32 {
    let parts: Vec<u32> = mc_version
        .strip_prefix("1.")
        .unwrap_or(mc_version)
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    if parts.is_empty() {
        return 8;
    }
    let major = parts[0];
    let minor = parts.get(1).copied().unwrap_or(0);
    if major >= 26 {
        25
    } else if major >= 21 || (major == 20 && minor >= 5) {
        21
    } else if major >= 17 {
        17
    } else {
        8
    }
}

/// 把用户自定义 JVM 参数字符串拆成单独参数：支持双引号包裹含空格的参数
/// （如 `-javaagent:"C:\Program Files\x.jar"`），未加引号时按空白拆分；
/// 换行也作为分隔符，方便一行一个参数地填写。比 split_whitespace 更稳。
fn split_jvm_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn append_jar_args(command: &mut Command, max: &str, min: &str, extra: Option<&str>, jar: &str) {
    command.args([max, min]);
    if let Some(extra) = extra {
        command.args(split_jvm_args(extra));
    }
    command.args(["-jar", jar, "nogui"]);
}

pub async fn start_server(
    app: tauri::AppHandle,
    state: &AppState,
    id: &str,
) -> Result<u32, String> {
    validate_server_id(id)?;
    let process = state.jobs.reserve_server(id)?;
    let server = get_server(state, id)?;
    let dir = server_dir(state, id);

    if !server.eula {
        return Err("需要先同意 Minecraft EULA 才能启动服务器".into());
    }
    let is_forge = server.core == ServerCore::Forge;
    let is_fabric = server.core == ServerCore::Fabric;
    let launch_jar = if is_fabric {
        find_launcher_jar(&dir, "fabric-server-")
            .ok_or_else(|| "未找到 Fabric launcher jar，请先安装核心".to_string())?
    } else {
        "server.jar".to_string()
    };
    let jar = dir.join(&launch_jar);
    let args_file = if is_forge { find_args_file(&dir) } else { None };
    if !is_forge && !jar.exists() {
        return Err(format!("{} 不存在，请先安装服务器核心", launch_jar));
    }
    if is_forge && args_file.is_none() && !jar.exists() {
        return Err("服务器核心未安装，请先安装核心".into());
    }

    // Fabric: 若原版 server.jar 缺失，自动补下载
    if is_fabric {
        let vanilla = dir.join("server.jar");
        if !vanilla.exists() {
            let _ = app.emit("server://log", serde_json::json!({
                "serverId": id, "stream": "out", "line": "[QookiX] 检测到原版服务端核心缺失，正在补下载，请稍候…"
            }));
            let vj = crate::mcmeta::fetch_version_json(state, &server.mc_version).await?;
            if let Some(dl) = vj.downloads.server {
                download_file_to_with_progress(
                    &app, &state.client, &dl.url, &vanilla, id, "正在补下载原版服务端核心…", None,
                )
                .await?;
                let _ = app.emit("server://log", serde_json::json!({
                    "serverId": id, "stream": "out", "line": "[QookiX] 原版服务端核心下载完成，正在启动服务器…"
                }));
            }
        }
    }

    ensure_server_files(&dir, server.eula)?;
    let (java, required_java) = pick_java(state, &jar, &server.mc_version, server.java_path.as_deref()).await?;
    let _ = app.emit("server://log", serde_json::json!({
        "serverId": id, "stream": "out", "line": format!("[QookiX] MC 版本 {}，需要 Java {}+，使用 Java {} (版本 {})：{}", server.mc_version, required_java, java.major, java.version, java.path)
    }));

    let max = format!("-Xmx{}M", server.max_memory_mb);
    let min = format!("-Xms{}M", server.min_memory_mb);
    let mut cmd = if let Some(ref args) = args_file {
        // Forge/NeoForge: 写入 JVM 参数到 user_jvm_args.txt，用 @argfile 启动
        let user_jvm = dir.join("user_jvm_args.txt");
        let mut jvm_content = format!("{}\n{}", max, min);
        if let Some(extra) = &server.jvm_args {
            jvm_content.push('\n');
            jvm_content.push_str(extra);
        }
        let _ = crate::fsutil::fs_best_effort("write", &user_jvm, std::fs::write(&user_jvm, &jvm_content));
        let mut c = Command::new(&java.path);
        c.arg(format!("@{}", user_jvm.to_string_lossy()))
            .arg(format!("@{}", args.display()))
            .arg("nogui")
            .current_dir(&dir);
        c
    } else {
        let mut c = Command::new(&java.path);
        append_jar_args(&mut c, &max, &min, server.jvm_args.as_deref(), &launch_jar);
        c.current_dir(&dir);
        c
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if process.cancel.is_cancelled() {
        return Err("服务器启动已取消".into());
    }
    let child = tokio::process::Command::from(cmd)
        .spawn()
        .map_err(|e| format!("启动 Java 失败: {e}"))?;
    let pid = child.id().unwrap_or(0);
    let task_id = process.task_id;
    process.set_pid(pid);
    let _ = app.emit(
        "server://state",
        serde_json::json!({ "serverId": id, "taskId": task_id, "state": "running", "pid": pid, "code": null }),
    );

    // mark started
    {
        if let Ok(mut s) = get_server(state, id) {
            s.last_started = Some(crate::util::now_secs());
            let _ = crate::util::log_best_effort("save_server", save_server(state, &s));
        }
    }

    // stream logs in background
    let app2 = app.clone();
    let sid = id.to_string();
    let log_path = dir.join("console.log");
    let stop_command = server.stop_command.unwrap_or_else(|| "stop".to_string());
    tauri::async_runtime::spawn(async move {
        let (outcome, tail) =
            stream_server_output(&app2, child, &sid, log_path, &stop_command, &process.cancel).await;
        if !process.cancel.is_cancelled() {
            if let Some(code) = outcome.filter(|code| *code != 0) {
                let _ = app2.emit(
                    "server://error",
                    serde_json::json!({ "serverId": &sid, "code": code, "tail": tail }),
                );
            }
        }
        let _ = app2.emit(
            "server://state",
            serde_json::json!({ "serverId": sid, "taskId": task_id, "state": "exited", "pid": pid, "code": outcome }),
        );
        drop(process);
    });

    Ok(pid)
}

async fn forward_console_lines(
    stream: &'static str,
    reader: impl tokio::io::AsyncRead + Unpin,
    tx: tokio::sync::mpsc::Sender<(&'static str, String)>,
) {
    let mut reader = tokio::io::BufReader::new(reader);
    while let Ok(Some(line)) = crate::server_logs::read_line(&mut reader).await {
        if tx.send((stream, line)).await.is_err() {
            break;
        }
    }
}

async fn record_console_lines(
    app: tauri::AppHandle,
    server_id: String,
    path: PathBuf,
    mut rx: tokio::sync::mpsc::Receiver<(&'static str, String)>,
) -> Vec<String> {
    let mut log = match crate::server_logs::DiskLog::open(path).await {
        Ok(log) => Some(log),
        Err(err) => {
            eprintln!("[server] 无法打开控制台日志: {err}");
            None
        }
    };
    let mut tail = std::collections::VecDeque::with_capacity(15);
    let mut flush = tokio::time::interval(std::time::Duration::from_millis(500));
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let result = tokio::select! {
            item = rx.recv() => {
                let Some((stream, line)) = item else { break };
                let _ = app.emit("server://log", serde_json::json!({
                    "serverId": &server_id, "stream": stream, "line": &line
                }));
                let written = if let Some(log) = &mut log { log.append(&line).await } else { Ok(()) };
                if tail.len() == 15 { tail.pop_front(); }
                tail.push_back(line);
                written
            }
            _ = flush.tick(), if log.is_some() => {
                if let Some(log) = &mut log { log.flush().await } else { Ok(()) }
            }
        };
        if let Err(err) = result {
            eprintln!("[server] 无法写入控制台日志: {err}");
            log = None;
        }
    }
    if let Some(log) = &mut log {
        if let Err(err) = log.flush().await {
            eprintln!("[server] 无法刷新控制台日志: {err}");
        }
    }
    tail.into_iter().collect()
}

async fn stream_server_output(
    app: &tauri::AppHandle,
    mut child: tokio::process::Child,
    server_id: &str,
    log_path: PathBuf,
    stop_command: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> (Option<i32>, Vec<String>) {
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let mut output = tokio::spawn(record_console_lines(app.clone(), server_id.to_owned(), log_path, rx));
    let mut tasks = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        tasks.push(tokio::spawn(forward_console_lines("out", stdout, tx.clone())));
    }
    if let Some(stderr) = child.stderr.take() {
        tasks.push(tokio::spawn(forward_console_lines("err", stderr, tx.clone())));
    }
    drop(tx);
    let mut stdin = child.stdin.take();
    let status = tokio::select! {
        status = child.wait() => status,
        _ = cancel.cancelled() => {
            if let Some(stdin) = stdin.as_mut() {
                use tokio::io::AsyncWriteExt;
                let command = format!("{stop_command}\n");
                let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                    stdin.write_all(command.as_bytes()).await?;
                    stdin.flush().await
                }).await;
                if !matches!(result, Ok(Ok(()))) {
                    let _ = app.emit("server://log", serde_json::json!({
                        "serverId": server_id, "stream": "err",
                        "line": "发送关服命令失败，仍在等待服务器退出",
                    }));
                }
            }
            child.wait().await
        }
    }.ok();
    for mut task in tasks {
        if tokio::time::timeout(std::time::Duration::from_secs(2), &mut task).await.is_err() {
            task.abort();
        }
    }
    let recent = match tokio::time::timeout(std::time::Duration::from_secs(4), &mut output).await {
        Ok(Ok(tail)) => tail,
        Ok(Err(err)) => {
            eprintln!("[server] 控制台日志任务失败: {err}");
            Vec::new()
        }
        Err(_) => {
            output.abort();
            Vec::new()
        }
    };
    let code = status.map(|s| s.code().unwrap_or(-1));
    let tail = if code.is_some_and(|code| code != 0) { recent } else { Vec::new() };
    (code, tail)
}

pub async fn stop_server(state: &AppState, id: &str) -> Result<(), String> {
    validate_server_id(id)?;
    let Some(finished) = state.jobs.request_server_shutdown(id) else {
        return Ok(());
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), finished.cancelled())
        .await
        .map_err(|_| "服务器尚未退出，请查看控制台日志".into())
}

pub fn is_server_running(state: &AppState, id: &str) -> bool {
    state.jobs.server_running(id)
}

pub async fn read_server_log(state: &AppState, id: &str) -> Result<Vec<String>, String> {
    validate_server_id(id)?;
    let path = server_dir(state, id).join("console.log");
    tokio::task::spawn_blocking(move || crate::server_logs::tail(&path))
        .await.map_err(|err| format!("读取日志任务失败: {err}"))?
        .map_err(|err| format!("读取日志失败: {err}"))
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

pub const SERVER_SUBFOLDERS: [&str; 9] = [
    "mods", "plugins", "config", "logs", "world", "world_nether", "world_the_end", "cache", "data",
];

pub fn list_server_folders(state: &AppState, id: &str) -> Vec<(String, bool)> {
    let dir = server_dir(state, id);
    SERVER_SUBFOLDERS
        .iter()
        .map(|f| (f.to_string(), dir.join(f).is_dir()))
        .collect()
}

pub fn resolve_server_path(
    state: &AppState,
    id: &str,
    rel: &str,
) -> Result<PathBuf, String> {
    validate_server_id(id)?;
    crate::fsutil::resolve_in_dir(&server_dir(state, id), rel, "服务器")
}

// ---------------------------------------------------------------------------
// Config file discovery
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone, Debug)]
pub struct ServerConfigFile {
    pub name: String,
    pub rel: String,
    pub size: u64,
    pub modified: u64,
}

fn is_config_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".properties")
        || lower.ends_with(".yml")
        || lower.ends_with(".yaml")
        || lower.ends_with(".json")
        || lower.ends_with(".txt")
        || lower.ends_with(".conf")
        || lower.ends_with(".toml")
}

fn meta_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Scan the server root and `config/` (up to 3 levels deep) for editable
/// configuration files. Returns entries with relative paths usable by
/// `read_hosted_server_file` / `write_hosted_server_file`.
pub fn list_server_config_files(
    state: &AppState,
    id: &str,
) -> Result<Vec<ServerConfigFile>, String> {
    let dir = server_dir(state, id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out: Vec<ServerConfigFile> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let path = e.path();
            if !path.is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if is_config_file(&name) {
                if let Ok(meta) = e.metadata() {
                    out.push(ServerConfigFile {
                        name: name.clone(),
                        rel: name,
                        size: meta.len(),
                        modified: meta_secs(&meta),
                    });
                }
            }
        }
    }

    scan_config_subdir(&dir.join("config"), "config", &mut out, 3);

    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn scan_config_subdir(base: &Path, prefix: &str, out: &mut Vec<ServerConfigFile>, depth: u32) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let rel = format!("{}/{}", prefix, name);
        if path.is_file() {
            if is_config_file(&name) {
                if let Ok(meta) = e.metadata() {
                    out.push(ServerConfigFile {
                        name,
                        rel,
                        size: meta.len(),
                        modified: meta_secs(&meta),
                    });
                }
            }
        } else if path.is_dir() {
            scan_config_subdir(&path, &rel, out, depth - 1);
        }
    }
}


#[cfg(test)]
mod launch_tests {
    use super::*;

    #[test]
    fn direct_server_launch_keeps_custom_jvm_options_before_jar() {
        let mut command = Command::new("java");
        append_jar_args(&mut command, "-Xmx4096M", "-Xms1024M", Some("-XX:+UseG1GC -Dkey=quoted"), "server.jar");
        let args = command.get_args().map(|value| value.to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(args, ["-Xmx4096M", "-Xms1024M", "-XX:+UseG1GC", "-Dkey=quoted", "-jar", "server.jar", "nogui"]);
    }
}
