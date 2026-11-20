//! 内网穿透（frp）内置支持。
//!
//! 只做三件事：设备授权登录 → 列出账号下的隧道 → 拉起/停止 `frpc` 子进程。
//! 隧道本身在面板上创建（远端端口、带宽套餐都由面板决定），启动器不碰这些规则。

pub mod api;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use serde_json::json;
use tauri::State;

use crate::state::AppState;

/// frpc 子进程的生命周期绑定。
///
/// Windows 默认不会随父进程退出杀掉子进程：启动器一旦崩溃/被结束，frpc 会变成孤儿，
/// 隧道还通着，但启动器重启后那个 `Child` 句柄已经丢了——UI 显示「未启动」，点启动又会
/// 拉第二个、报 `proxy already exists`，而且停不掉。
///
/// 这里用 Job Object + `KILL_ON_JOB_CLOSE`：Job 句柄只活在启动器进程内，
/// 进程无论怎么退出（正常 / 崩溃 / 任务管理器结束），系统都会把 Job 里的子进程一起结束。
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

    fn job_handle() -> Option<HANDLE> {
        unsafe {
            let cached = JOB.load(Ordering::SeqCst);
            if !cached.is_null() {
                return Some(cached);
            }
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let _ = CloseHandle(h);
                return None;
            }
            JOB.store(h, Ordering::SeqCst);
            Some(h)
        }
    }

    /// 把刚 spawn 的子进程收进 Job
    pub fn adopt(child: &std::process::Child) {
        unsafe {
            if let Some(h) = job_handle() {
                let _ = AssignProcessToJobObject(h, child.as_raw_handle() as HANDLE);
            }
        }
    }
}

#[cfg(not(windows))]
mod job {
    pub fn adopt(_child: &std::process::Child) {}
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TunnelKey {
    Panel(i64),
    Local(String),
}

impl TunnelKey {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.split_once(':') {
            Some(("panel", id)) => id.parse::<i64>().ok().filter(|id| *id > 0)
                .map(Self::Panel).ok_or_else(|| "面板隧道 ID 无效".into()),
            Some(("local", id)) => {
                crate::fsutil::validate_id(id, "本地隧道")?;
                Ok(Self::Local(id.into()))
            }
            _ => Err("隧道来源或 ID 无效".into()),
        }
    }

    fn wire_id(&self) -> String {
        match self {
            Self::Panel(id) => format!("panel:{id}"),
            Self::Local(id) => format!("local:{id}"),
        }
    }

    fn log_path(&self, root: &Path) -> PathBuf {
        let logs = root.join("logs");
        match self {
            Self::Panel(id) => logs.join("panel").join(format!("{id}.log")),
            Self::Local(id) => logs.join("local").join(format!("{id}.log")),
        }
    }
}

#[derive(Default)]
struct Runtime {
    generation: u64,
    pending: Option<api::DeviceAuth>,
    processes: HashMap<TunnelKey, Child>,
}

impl Runtime {
    fn check_panel(&self, generation: u64) -> Result<(), String> {
        (self.generation == generation).then_some(()).ok_or_else(|| "面板会话已变化，请重试".into())
    }

    fn reap(&mut self) {
        self.processes.retain(|_, child| !matches!(child.try_wait(), Ok(Some(_))));
    }

    fn stop(&mut self, key: &TunnelKey) -> Result<(), String> {
        let Some(child) = self.processes.get_mut(key) else { return Ok(()) };
        if child.try_wait().map_err(|err| err.to_string())?.is_none() {
            child.kill().map_err(|err| format!("停止 frpc 失败: {err}"))?;
            child.wait().map_err(|err| format!("回收 frpc 失败: {err}"))?;
        }
        self.processes.remove(key);
        Ok(())
    }

    fn stop_matching(&mut self, matches: impl Fn(&TunnelKey) -> bool) -> Result<(), String> {
        let keys: Vec<_> = self.processes.keys().filter(|key| matches(key)).cloned().collect();
        let mut first_error = None;
        for key in keys {
            if let Err(err) = self.stop(&key) {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

fn runtime() -> std::sync::MutexGuard<'static, Runtime> {
    static RUNTIME: std::sync::LazyLock<Mutex<Runtime>> = std::sync::LazyLock::new(|| Mutex::new(Runtime::default()));
    RUNTIME.lock().unwrap_or_else(|error| error.into_inner())
}

fn auth_path(state: &AppState) -> std::path::PathBuf {
    state.root.join("frp_auth.json")
}

fn frp_dir(state: &AppState) -> std::path::PathBuf {
    state.root.join("frp")
}

/// 读本地 Token（DPAPI 加密）
fn load_token(state: &AppState) -> Option<String> {
    let text = std::fs::read_to_string(auth_path(state)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let raw = v.get("token").and_then(|t| t.as_str())?;
    crate::secret::unprotect(raw).ok().filter(|s| !s.is_empty())
}

fn save_token(state: &AppState, token: &str) -> Result<(), String> {
    let encrypted = crate::secret::protect(token)?;
    let bytes = serde_json::to_vec_pretty(&json!({ "token": encrypted })).map_err(|err| err.to_string())?;
    crate::fsutil::atomic_write(&auth_path(state), &bytes).map_err(|err| format!("保存登录态失败: {err}"))
}

fn panel_token(state: &AppState) -> Option<(u64, String)> {
    let runtime = runtime();
    load_token(state).map(|token| (runtime.generation, token))
}

fn start_process(runtime: &mut Runtime, state: &AppState, key: TunnelKey, config: &Path) -> Result<PathBuf, String> {
    runtime.reap();
    let log_path = key.log_path(&frp_dir(state));
    if runtime.processes.contains_key(&key) {
        return Ok(log_path);
    }
    let frpc = find_frpc(state)?;
    std::fs::create_dir_all(log_path.parent().ok_or("日志目录无效")?).map_err(|err| err.to_string())?;
    let log = std::fs::File::create(&log_path).map_err(|err| format!("建立 frpc 日志失败: {err}"))?;
    let stderr = log.try_clone().map_err(|err| err.to_string())?;
    let child = Command::new(frpc).arg("-c").arg(config).stdin(Stdio::null())
        .stdout(Stdio::from(log)).stderr(Stdio::from(stderr)).spawn()
        .map_err(|err| format!("启动 frpc 失败: {err}"))?;
    job::adopt(&child);
    runtime.processes.insert(key, child);
    Ok(log_path)
}

/// 找 frpc 可执行文件：优先 `<数据目录>/frp/frpc.exe`，再退回 exe 同目录
fn find_frpc(state: &AppState) -> Result<std::path::PathBuf, String> {
    let mut cands = vec![frp_dir(state).join("frpc.exe")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("frpc.exe"));
        }
    }
    let want = cands
        .first()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    for c in cands {
        if c.is_file() {
            return Ok(c);
        }
    }
    // 把期望路径直接给出来，用户不用猜「数据目录」是哪
    Err(format!(
        "缺少 frpc.exe。请把 frp 官方发行包里的 frpc.exe 放到：\n{want}"
    ))
}

// ------------------------------------------------------------------ 命令

/// 开始设备授权：返回授权码与链接，前端展示并引导用户去浏览器
#[tauri::command]
pub async fn frp_login_start() -> Result<serde_json::Value, String> {
    let generation = {
        let mut runtime = runtime();
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.pending = None;
        runtime.generation
    };
    let d = api::device_register().await?;
    let mut runtime = runtime();
    runtime.check_panel(generation)?;
    runtime.pending = Some(d.clone());
    Ok(json!({
        "userCode": d.user_code,
        "uri": d.verification_uri_complete,
        "interval": d.interval,
        "expiresIn": d.expires_in,
    }))
}

/// 轮询一次（**前端负责按 interval 调用**，这里不 sleep，避免占用运行时）
#[tauri::command]
pub async fn frp_login_poll(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (generation, device_code) = {
        let runtime = runtime();
        let device = runtime.pending.as_ref().ok_or("没有进行中的授权，请重新获取授权码")?;
        (runtime.generation, device.device_code.clone())
    };
    match api::device_token(&device_code).await? {
        api::PollResult::Pending => {
            runtime().check_panel(generation)?;
            Ok(json!({ "status": "pending" }))
        }
        api::PollResult::Authorized(token) => {
            let user = api::user_balance(&token).await.ok();
            let mut runtime = runtime();
            runtime.check_panel(generation)?;
            if runtime.pending.as_ref().is_none_or(|device| device.device_code != device_code) {
                return Err("设备授权已失效".into());
            }
            runtime.stop_matching(|key| matches!(key, TunnelKey::Panel(_)))?;
            save_token(&state, &token)?;
            runtime.pending = None;
            runtime.generation = runtime.generation.wrapping_add(1);
            Ok(json!({ "status": "authorized", "user": user }))
        }
    }
}

/// 当前登录态
#[tauri::command]
pub async fn frp_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let Some((generation, token)) = panel_token(&state) else {
        return Ok(json!({ "loggedIn": false, "user": null }));
    };
    let response = api::user_balance(&token).await;
    runtime().check_panel(generation)?;
    match response {
        Ok(user) => Ok(json!({ "loggedIn": true, "user": user })),
        Err(err) => Ok(json!({ "loggedIn": true, "user": null, "warning": err })),
    }
}

#[tauri::command]
pub async fn frp_tunnels(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (generation, token) = panel_token(&state).ok_or("未登录面板")?;
    let tunnels = api::list_tunnels(&token).await?;
    let mut runtime = runtime();
    runtime.check_panel(generation)?;
    runtime.reap();
    let running: Vec<_> = runtime.processes.keys().filter(|key| matches!(key, TunnelKey::Panel(_)))
        .map(TunnelKey::wire_id).collect();
    Ok(json!({ "tunnels": tunnels, "running": running }))
}

#[tauri::command]
pub async fn frp_start(state: State<'_, AppState>, tunnel_id: i64) -> Result<serde_json::Value, String> {
    if tunnel_id <= 0 { return Err("面板隧道 ID 无效".into()); }
    let key = TunnelKey::Panel(tunnel_id);
    let (generation, token) = panel_token(&state).ok_or("未登录面板")?;
    let cfg = api::tunnel_config(&token, tunnel_id).await?;
    if cfg.config.trim().is_empty() {
        return Err("面板没有返回可用的隧道配置".into());
    }
    let name = if cfg.tunnel_name.is_empty() { format!("tunnel_{tunnel_id}") } else { cfg.tunnel_name };
    let dir = frp_dir(&state).join("panel");
    let mut runtime = runtime();
    runtime.check_panel(generation)?;
    let path = dir.join(format!("{tunnel_id}.toml"));
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    crate::fsutil::atomic_write(&path, cfg.config.as_bytes()).map_err(|err| format!("写入隧道配置失败: {err}"))?;
    let run_id = key.wire_id();
    let log = start_process(&mut runtime, &state, key, &path)?;
    Ok(json!({ "runId": run_id, "name": name, "log": log }))
}

#[tauri::command]
pub fn frp_stop(run_id: String) -> Result<(), String> {
    runtime().stop(&TunnelKey::parse(&run_id)?)
}

/// 去掉 ANSI 颜色转义（`\x1b[...m` 之类），日志要给人看而不是给终端
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for n in it.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// 读取隧道日志尾部（默认 300 行）
#[tauri::command]
pub async fn frp_logs(state: State<'_, AppState>, run_id: String, lines: Option<usize>) -> Result<String, String> {
    let path = TunnelKey::parse(&run_id)?.log_path(&frp_dir(&state));
    tokio::task::spawn_blocking(move || {
        if !path.is_file() { return Err("还没有日志（这条隧道可能没启动过）".into()); }
        let all = crate::server_logs::tail(&path).map_err(|err| format!("读取隧道日志失败: {err}"))?;
        let start = all.len().saturating_sub(lines.unwrap_or(300).clamp(1, 2000));
        let text = strip_ansi(&all[start..].join("\n"));
        if text.trim().is_empty() { return Ok("（日志为空，可能是进程刚启动就被拒绝了）".into()); }
        Ok(text)
    }).await.map_err(|err| err.to_string())?
}

/// 面板退出时只停止该来源的隧道。
#[tauri::command]
pub fn frp_logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut runtime = runtime();
    match std::fs::remove_file(auth_path(&state)) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("清除面板登录失败: {err}")),
    }
    runtime.generation = runtime.generation.wrapping_add(1);
    runtime.pending = None;
    runtime.stop_matching(|key| matches!(key, TunnelKey::Panel(_)))
}

pub fn cleanup() {
    if let Err(err) = runtime().stop_matching(|_| true) {
        eprintln!("[frp] {err}");
    }
}

// ------------------------------------------------------------------ 本地自定义隧道

/// 本地隧道（不经过面板创建，配置由启动器自己拼）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalTunnel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub running: bool,
    pub name: String,
    pub protocol: String,
    pub local_port: u32,
    pub remote_port: u32,
    pub host: String,
    pub port: u32,
}

/// 本地隧道配置的自有标记：只有带这行的文件才算「本地隧道」，
/// 既能过滤面板下发的老文件，也方便以后排查。
const LOCAL_MARKER: &str = "本地自定义隧道";
const LABEL_MARKER: &str = "# qookix-name: ";

fn local_dir(state: &AppState) -> std::path::PathBuf {
    frp_dir(state).join("tunnels")
}

/// 列出本地隧道（从 `frp/tunnels/*.toml` 反推）
#[tauri::command]
pub fn frp_local_list(state: State<'_, AppState>) -> Result<Vec<LocalTunnel>, String> {
    let dir = local_dir(&state);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut runtime = runtime();
    runtime.reap();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") || !e.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        // 史上面板隧道也写进过这个目录，按标记过滤掉
        if !text.contains(LOCAL_MARKER) {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if crate::fsutil::validate_id(id, "本地隧道").is_err() { continue; }
        let name = text.lines().find_map(|line| line.strip_prefix(LABEL_MARKER))
            .and_then(|line| serde_json::from_str::<String>(line).ok())
            .unwrap_or_else(|| id.to_owned());
        let pick = |key: &str| -> String {
            text.lines()
                .find_map(|l| {
                    let l = l.trim();
                    l.strip_prefix(key)
                        .map(|r| r.trim().trim_start_matches('=').trim().trim_matches('"').to_string())
                })
                .unwrap_or_default()
        };
        out.push(LocalTunnel {
            id: id.into(),
            running: runtime.processes.contains_key(&TunnelKey::Local(id.into())),
            name,
            protocol: {
                let p = pick("type");
                if p.is_empty() { "tcp".into() } else { p }
            },
            local_port: pick("localPort").parse().unwrap_or(0),
            remote_port: pick("remotePort").parse().unwrap_or(0),
            host: pick("serverAddr"),
            port: pick("serverPort").parse().unwrap_or(0),
        });
    }
    Ok(out)
}

fn local_config(state: &AppState, id: &str) -> Result<PathBuf, String> {
    crate::fsutil::validate_id(id, "本地隧道")?;
    let path = local_dir(state).join(format!("{id}.toml"));
    if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
        return Err("隧道配置不存在".into());
    }
    let text = std::fs::read_to_string(&path).map_err(|err| format!("读取隧道配置失败: {err}"))?;
    if !text.lines().next().is_some_and(|line| line.starts_with('#') && line.contains(LOCAL_MARKER)) {
        return Err("该文件不是启动器的本地隧道".into());
    }
    Ok(path)
}

#[tauri::command]
pub fn frp_local_save(state: State<'_, AppState>, tunnel: LocalTunnel) -> Result<String, String> {
    let _runtime = runtime();
    let token = load_token(&state).ok_or("请先登录面板（需要它作为 frp 认证 token）")?;
    let host = tunnel.host.trim();
    if host.is_empty() || host.len() > 255 || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("节点地址无效".into());
    }
    if [tunnel.port, tunnel.local_port, tunnel.remote_port].into_iter().any(|port| !(1..=65535).contains(&port)) {
        return Err("端口必须在 1 到 65535 之间".into());
    }
    let label = tunnel.name.trim();
    if label.is_empty() || label.chars().count() > 128 {
        return Err("隧道名称必须为 1 到 128 个字符".into());
    }
    let dir = local_dir(&state);
    let (id, path) = if tunnel.id.is_empty() {
        let id = format!("l-{}", uuid::Uuid::new_v4().simple());
        let path = dir.join(format!("{id}.toml"));
        (id, path)
    } else {
        let path = local_config(&state, &tunnel.id)?;
        (tunnel.id, path)
    };
    let proto = if tunnel.protocol == "udp" { "udp" } else { "tcp" };
    let quote = |value: &str| serde_json::to_string(value).map_err(|err| err.to_string());
    let toml = format!(r#"# 本地自定义隧道（启动器生成）
{LABEL_MARKER}{label}
serverAddr = {host}
serverPort = {port}
auth.method = "token"
auth.token = {token}

[log]
to = "console"
level = "info"

[[proxies]]
name = {name}
type = "{proto}"
localIP = "127.0.0.1"
localPort = {local}
remotePort = {remote}
"#, label = quote(label)?, host = quote(host)?, port = tunnel.port, token = quote(&token)?,
        name = quote(&id)?, local = tunnel.local_port, remote = tunnel.remote_port);
    std::fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    crate::fsutil::atomic_write(&path, toml.as_bytes()).map_err(|err| format!("写入隧道配置失败: {err}"))?;
    Ok(id)
}

#[tauri::command]
pub fn frp_local_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut runtime = runtime();
    let path = local_config(&state, &id)?;
    runtime.stop(&TunnelKey::Local(id))?;
    std::fs::remove_file(path).map_err(|err| format!("移除隧道配置失败: {err}"))
}

#[tauri::command]
pub fn frp_local_start(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut runtime = runtime();
    let path = local_config(&state, &id)?;
    start_process(&mut runtime, &state, TunnelKey::Local(id), &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_source_and_id_determine_logs_independent_of_labels() {
        let remote = TunnelKey::parse("panel:7").unwrap();
        let local = TunnelKey::parse("local:7").unwrap();
        assert_ne!(local, remote);
        assert_ne!(local.log_path(Path::new("frp")), remote.log_path(Path::new("frp")));
        assert_ne!(remote, TunnelKey::parse("panel:8").unwrap());
        assert!(TunnelKey::parse("local:../other").is_err());
    }

    #[test]
    #[ignore = "subprocess fixture"]
    fn owned_test_tunnel() {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }

    #[test]
    fn panel_logout_keeps_a_running_local_tunnel_with_the_same_id() {
        let launch = || Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "frp::tests::owned_test_tunnel"])
            .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let mut runtime = Runtime::default();
        let panel = TunnelKey::Panel(7);
        let local = TunnelKey::Local("7".into());
        runtime.processes.insert(panel.clone(), launch());
        runtime.processes.insert(local.clone(), launch());
        let result = runtime.stop_matching(|key| matches!(key, TunnelKey::Panel(_)));
        let panel_stopped = !runtime.processes.contains_key(&panel);
        let local_running = runtime.processes.get_mut(&local)
            .is_some_and(|child| matches!(child.try_wait(), Ok(None)));
        runtime.stop_matching(|_| true).unwrap();
        result.unwrap();
        assert!(panel_stopped);
        assert!(local_running);
    }
}
