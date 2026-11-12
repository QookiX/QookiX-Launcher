//! 内网穿透（frp）内置支持。
//!
//! 只做三件事：设备授权登录 → 列出账号下的隧道 → 拉起/停止 `frpc` 子进程。
//! 隧道本身在面板上创建（远端端口、带宽套餐都由面板决定），启动器不碰这些规则。

pub mod api;

use std::collections::HashMap;
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

/// 等待授权的会话（device_code 不能展示给用户，放内存里即可）
static PENDING: Mutex<Option<api::DeviceAuth>> = Mutex::new(None);
/// 正在跑的隧道：隧道名 → 子进程
static RUNNING: Mutex<Option<HashMap<String, Child>>> = Mutex::new(None);

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
    let enc = crate::secret::protect(token)?;
    let text = serde_json::to_string_pretty(&json!({ "token": enc })).map_err(|e| e.to_string())?;
    std::fs::write(auth_path(state), text).map_err(|e| format!("保存登录态失败: {e}"))
}

/// 隧道名做文件名时只留安全字符
fn safe_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if s.is_empty() {
        format!("tunnel_{}", crate::util::now_secs())
    } else {
        s
    }
}

fn running_map() -> std::sync::MutexGuard<'static, Option<HashMap<String, Child>>> {
    RUNNING.lock().unwrap_or_else(|e| e.into_inner())
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
    let d = api::device_register().await?;
    let interval = d.interval;
    let expires_in = d.expires_in;
    {
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        *p = Some(d.clone());
    }
    Ok(json!({
        "userCode": d.user_code,
        "uri": d.verification_uri_complete,
        "interval": interval,
        "expiresIn": expires_in,
    }))
}

/// 轮询一次（**前端负责按 interval 调用**，这里不 sleep，避免占用运行时）
#[tauri::command]
pub async fn frp_login_poll(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let device_code = {
        let p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        p.as_ref()
            .map(|d| d.device_code.clone())
            .ok_or_else(|| "没有进行中的授权，请重新获取授权码".to_string())?
    };
    match api::device_token(&device_code).await? {
        api::PollResult::Pending => Ok(json!({ "status": "pending" })),
        api::PollResult::Authorized(token) => {
            save_token(&state, &token)?;
            {
                let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
                *p = None;
            }
            let user = api::user_balance(&token).await.ok();
            Ok(json!({ "status": "authorized", "user": user }))
        }
    }
}

/// 当前登录态
#[tauri::command]
pub async fn frp_status(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let Some(token) = load_token(&state) else {
        return Ok(json!({ "loggedIn": false, "user": null }));
    };
    match api::user_balance(&token).await {
        Ok(u) => Ok(json!({ "loggedIn": true, "user": u })),
        // 网络问题不该把用户踢下线，保持登录态但标记未校验
        Err(e) => Ok(json!({ "loggedIn": true, "user": null, "warning": e })),
    }
}

/// 隧道列表（附带本地是否正在跑）
#[tauri::command]
pub async fn frp_tunnels(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let token = load_token(&state).ok_or_else(|| "未登录面板".to_string())?;
    let tunnels = api::list_tunnels(&token).await?;
    let running: Vec<String> = {
        let mut m = running_map();
        match m.as_mut() {
            Some(map) => {
                // 顺手回收已经退出的进程
                map.retain(|_, c| matches!(c.try_wait(), Ok(None)));
                map.keys().cloned().collect()
            }
            None => Vec::new(),
        }
    };
    Ok(json!({ "tunnels": tunnels, "running": running }))
}

/// 启动一条隧道
#[tauri::command]
pub async fn frp_start(
    state: State<'_, AppState>,
    tunnel_id: i64,
) -> Result<serde_json::Value, String> {
    let token = load_token(&state).ok_or_else(|| "未登录面板".to_string())?;
    let cfg = api::tunnel_config(&token, tunnel_id).await?;
    if cfg.config.trim().is_empty() {
        return Err("面板没有返回可用的隧道配置".into());
    }
    let name = if cfg.tunnel_name.is_empty() {
        format!("tunnel_{tunnel_id}")
    } else {
        cfg.tunnel_name.clone()
    };
    let safe = safe_name(&name);

    let dir = frp_dir(&state);
    // 面板下发的配置放 panel/，和用户自己的本地隧道（tunnels/）分开，
    // 否则本地隧道列表里会混进一堆面板隧道
    let tun_dir = dir.join("panel");
    let log_dir = dir.join("logs");
    std::fs::create_dir_all(&tun_dir).map_err(|e| format!("建目录失败: {e}"))?;
    std::fs::create_dir_all(&log_dir).map_err(|e| format!("建目录失败: {e}"))?;

    let cfg_path = tun_dir.join(format!("{safe}.toml"));
    std::fs::write(&cfg_path, &cfg.config).map_err(|e| format!("写入配置失败: {e}"))?;
    let log_path = log_dir.join(format!("{safe}.log"));
    let log = std::fs::File::create(&log_path).map_err(|e| format!("建日志失败: {e}"))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;

    let frpc = find_frpc(&state)?;
    // 注意：frp 0.6x 没有 --no-banner，带上它 cobra 会把 -c 的值当成子命令，
    // 报 `unknown command "<config path>"`。只传 -c 即可。
    let child = Command::new(&frpc)
        .arg("-c")
        .arg(&cfg_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .spawn()
        .map_err(|e| format!("启动 frpc 失败: {e}"))?;
    // 绑进 Job：启动器无论怎么退出，frpc 都会跟着结束，不会留下管不到的孤儿
    job::adopt(&child);

    {
        let mut m = running_map();
        let map = m.get_or_insert_with(HashMap::new);
        // 同名隧道先停掉旧的
        if let Some(mut old) = map.remove(&safe) {
            let _ = old.kill();
        }
        map.insert(safe.clone(), child);
    }

    Ok(json!({ "name": safe, "log": log_path.to_string_lossy() }))
}

/// 停止一条隧道
///
/// 登记时用的是 `safe_name`（空格等会被替换），这里必须做同样转换，
/// 否则面板上带空格的隧道名永远匹配不上、停不掉。
#[tauri::command]
pub fn frp_stop(name: String) -> Result<(), String> {
    let key = safe_name(&name);
    let mut m = running_map();
    if let Some(map) = m.as_mut() {
        if let Some(mut c) = map.remove(&key) {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    Ok(())
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
pub fn frp_logs(
    state: State<'_, AppState>,
    name: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let path = frp_dir(&state)
        .join("logs")
        .join(format!("{}.log", safe_name(&name)));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "还没有日志（这条隧道可能没启动过）".to_string())?;
    let n = lines.unwrap_or(300).clamp(1, 2000);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(n);
    // frpc 往 stderr 写的是带颜色的日志，文件里留着 ESC 序列，直接显示是一堆 [1;34m
    let tail = strip_ansi(&all[start..].join("\n"));
    if tail.trim().is_empty() {
        return Ok("（日志为空，可能是进程刚启动就被拒绝了）".to_string());
    }
    Ok(tail)
}

/// 退出登录：清 Token + 停掉所有隧道
#[tauri::command]
pub fn frp_logout(state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut m = running_map();
        if let Some(map) = m.as_mut() {
            for (_, mut c) in map.drain() {
                let _ = c.kill();
            }
        }
    }
    let _ = std::fs::remove_file(auth_path(&state));
    Ok(())
}

/// 退出启动器时清理（由 lib.rs 的退出钩子调用）
pub fn cleanup() {
    let mut m = running_map();
    if let Some(map) = m.as_mut() {
        for (_, mut c) in map.drain() {
            let _ = c.kill();
        }
    }
}

// ------------------------------------------------------------------ 本地自定义隧道

/// 本地隧道（不经过面板创建，配置由启动器自己拼）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LocalTunnel {
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
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        // 史上面板隧道也写进过这个目录，按标记过滤掉
        if !text.contains(LOCAL_MARKER) {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
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

/// 新建或覆盖一条本地隧道：认证 token 用面板 JWT（文档 §8 的第三优先级）
#[tauri::command]
pub fn frp_local_save(state: State<'_, AppState>, tunnel: LocalTunnel) -> Result<String, String> {
    let token = load_token(&state).ok_or_else(|| "请先登录面板（需要它作为 frp 认证 token）".to_string())?;
    if tunnel.host.trim().is_empty() {
        return Err("节点地址不能为空".into());
    }
    if tunnel.local_port == 0 {
        return Err("本地端口不能为 0".into());
    }
    // 只允许 tcp/udp，避免拼出任意 frp 类型
    let proto = match tunnel.protocol.as_str() {
        "udp" => "udp",
        _ => "tcp",
    };
    let name = safe_name(&tunnel.name);

    let toml = format!(
        "# 本地自定义隧道（启动器生成）\n\
         serverAddr = \"{host}\"\n\
         serverPort = {port}\n\
         auth.method = \"token\"\n\
         auth.token = \"{token}\"\n\
         \n\
         [log]\n\
         to = \"console\"\n\
         level = \"info\"\n\
         \n\
         [[proxies]]\n\
         name = \"{name}\"\n\
         type = \"{proto}\"\n\
         localIP = \"127.0.0.1\"\n\
         localPort = {local}\n\
         remotePort = {remote}\n",
        host = tunnel.host.trim().replace('"', ""),
        port = tunnel.port,
        token = token.replace('"', ""),
        name = name,
        proto = proto,
        local = tunnel.local_port,
        remote = tunnel.remote_port,
    );

    let dir = local_dir(&state);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败: {e}"))?;
    std::fs::write(dir.join(format!("{name}.toml")), toml)
        .map_err(|e| format!("写入配置失败: {e}"))?;
    Ok(name)
}

/// 删除一条本地隧道（只删本地文件，不影响面板）
#[tauri::command]
pub fn frp_local_delete(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let path = local_dir(&state).join(format!("{}.toml", safe_name(&name)));
    let _ = std::fs::remove_file(path);
    Ok(())
}

/// 直接启动一个本地隧道文件
#[tauri::command]
pub fn frp_local_start(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let safe = safe_name(&name);
    let cfg_path = local_dir(&state).join(format!("{safe}.toml"));
    if !cfg_path.is_file() {
        return Err("隧道配置不存在".into());
    }
    let log_dir = frp_dir(&state).join("logs");
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(log_dir.join(format!("{safe}.log")))
        .map_err(|e| format!("建日志失败: {e}"))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    let frpc = find_frpc(&state)?;
    // 注意：frp 0.6x 没有 --no-banner，带上它 cobra 会把 -c 的值当成子命令，
    // 报 `unknown command "<config path>"`。只传 -c 即可。
    let child = Command::new(&frpc)
        .arg("-c")
        .arg(&cfg_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log2))
        .spawn()
        .map_err(|e| format!("启动 frpc 失败: {e}"))?;
    // 绑进 Job：启动器无论怎么退出，frpc 都会跟着结束，不会留下管不到的孤儿
    job::adopt(&child);
    let mut m = running_map();
    let map = m.get_or_insert_with(HashMap::new);
    if let Some(mut old) = map.remove(&safe) {
        let _ = old.kill();
    }
    map.insert(safe, child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_strips_separators() {
        assert_eq!(safe_name("my tunnel"), "my_tunnel");
        // 只替换字符，不删长度：`..` + `/` + `..` + `/` 共 6 个字符
        assert_eq!(safe_name("../../etc/passwd"), "______etc_passwd");
        assert!(!safe_name("").is_empty());
    }
}
