//! Yggdrasil（authlib-injector）皮肤站账号：登录、令牌保活与 authlib-injector 获取。
//!
//! 协议遵循 authlib-injector 规范的 authserver 端点，Blessing Skin 系皮肤站
//! （LittleSkin / 自建站）通用：
//! - `POST /authserver/authenticate` 登录
//! - `POST /authserver/validate` 校验令牌（204 即有效）
//! - `POST /authserver/refresh` 刷新令牌（rotate：旧令牌立即失效）
//! - `GET <root>` 返回站点 meta（serverName / 链接 / skinDomains）
//! - authlib-injector jar 从官方 artifact 接口获取（皮肤站 meta 不提供）

use crate::models::Account;
use crate::state::AppState;
use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 规整用户输入的 API root：去空白与尾部斜杠。
pub fn normalize_root(root: &str) -> String {
    root.trim().trim_end_matches('/').to_string()
}

async fn post_json(state: &AppState, url: &str, body: &Value) -> Result<(u16, Value), String> {
    let resp = state
        .client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("请求皮肤站失败: {e}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析皮肤站响应失败: {e}"))?;
    Ok((status, v))
}

/// 把皮肤站的错误响应转成用户可读的信息（Yggdrasil 约定 `errorMessage` 字段）。
fn server_error(status: u16, v: &Value, fallback: &str) -> String {
    let msg = v
        .get("errorMessage")
        .and_then(|m| m.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fallback.to_string());
    format!("{msg}（HTTP {status}）")
}

/// 站点主页 / 注册页链接（meta 的 `links` 字段，登录页展示用）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggLinks {
    pub homepage: String,
    pub register: String,
}

/// 登录结果：令牌 + 站点信息 + 可选角色列表（多角色时由用户挑选）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggLoginResult {
    pub access_token: String,
    pub client_token: String,
    /// 规整后的 API root
    pub server: String,
    /// 站点 meta 里的 serverName（如 "LittleSkin"）
    pub server_name: String,
    pub profiles: Vec<YggProfile>,
    pub links: YggLinks,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct YggProfile {
    pub id: String,
    pub name: String,
}

/// 拉取皮肤站账号的皮肤/披风纹理，下载为 data URL 供启动器内渲染头像。
/// 角色没有皮肤数据时返回 null。
#[tauri::command]
pub async fn yggdrasil_textures(
    state: tauri::State<'_, AppState>,
    server: String,
    profile_id: String,
) -> Result<Option<Value>, String> {
    let root = normalize_root(&server);
    let pid = profile_id.trim().to_string();
    if root.is_empty() || pid.is_empty() {
        return Ok(None);
    }
    let url = format!("{root}/sessionserver/session/minecraft/profile/{pid}");
    let resp = state
        .client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("访问皮肤站 sessionserver 失败: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // 该角色没有纹理数据
    }
    if !resp.status().is_success() {
        return Err(format!("拉取纹理失败（HTTP {}）", resp.status().as_u16()));
    }
    let profile: Value = resp.json().await.map_err(|e| e.to_string())?;
    let encoded = profile
        .get("properties")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("textures"))
        })
        .and_then(|p| p.get("value").and_then(|v| v.as_str()))
        .ok_or_else(|| "纹理数据格式异常".to_string())?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| format!("纹理数据 base64 解码失败: {e}"))?;
    let textures: Value = serde_json::from_slice(&decoded).map_err(|e| e.to_string())?;
    let skin_url = textures.pointer("/textures/SKIN/url").and_then(|v| v.as_str());
    let cape_url = textures.pointer("/textures/CAPE/url").and_then(|v| v.as_str());
    // 皮肤站图片不带 CORS 头，由后端下载转 data URL，前端 canvas 才能裁头
    async fn download(state: &AppState, url: &str) -> Result<Vec<u8>, String> {
        let bytes = state
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("下载纹理失败: {e}"))?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes.to_vec())
    }
    let encode =
        |bytes: &[u8]| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes));
    let skin = match skin_url {
        Some(u) => Some(encode(&download(&state, u).await?)),
        None => None,
    };
    let cape = match cape_url {
        Some(u) => Some(encode(&download(&state, u).await?)),
        None => None,
    };
    if skin.is_none() && cape.is_none() {
        return Ok(None);
    }
    Ok(Some(json!({ "skin": skin, "cape": cape })))
}

/// 登录皮肤站：验证账号密码，返回令牌与该账号下的全部角色。
/// 顺带拉取站点 meta，带出站点名与主页/注册链接（登录页展示用）。
#[tauri::command]
pub async fn yggdrasil_login(
    state: tauri::State<'_, AppState>,
    server_url: String,
    username: String,
    password: String,
) -> Result<YggLoginResult, String> {
    let root = normalize_root(&server_url);
    if root.is_empty() {
        return Err("请填写皮肤站 API 地址".into());
    }
    if username.trim().is_empty() || password.is_empty() {
        return Err("请填写账号和密码".into());
    }
    // 先拉站点 meta：拿 serverName 与主页/注册链接，顺带验证地址是否可达
    let meta: Value = crate::download::get_json(&state.client, &root)
        .await
        .map_err(|e| format!("无法访问皮肤站（{root}）: {e}"))?;
    let server_name = meta
        .pointer("/meta/serverName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("自定义皮肤站")
        .to_string();
    let links = YggLinks {
        homepage: meta
            .pointer("/links/homepage")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        register: meta
            .pointer("/links/register")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };

    let client_token = uuid::Uuid::new_v4().simple().to_string();
    let (status, body) = post_json(
        state.inner(),
        &format!("{root}/authserver/authenticate"),
        &json!({
            // 注意：Yggdrasil 规范要求 agent.name 恰为 "Minecraft"，
            // 写成 "MinecraftClient" 会被 Blessing Skin 系服务端以 400 拒绝
            "agent": { "name": "Minecraft", "version": 1 },
            "username": username.trim(),
            "password": password,
            "clientToken": client_token,
        }),
    )
    .await?;
    if status != 200 {
        return Err(server_error(
            status,
            &body,
            "登录失败，请检查账号、密码或皮肤站地址",
        ));
    }
    let access_token = body
        .get("accessToken")
        .and_then(|v| v.as_str())
        .ok_or_else(|| server_error(status, &body, "皮肤站未返回令牌"))?
        .to_string();
    let profiles: Vec<YggProfile> = body
        .get("availableProfiles")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    Some(YggProfile {
                        id: p.get("id").and_then(|v| v.as_str())?.to_string(),
                        name: p.get("name").and_then(|v| v.as_str())?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if profiles.is_empty() {
        return Err("该账号下没有角色，请先到皮肤站创建角色".into());
    }
    Ok(YggLoginResult {
        access_token,
        client_token,
        server: root,
        server_name,
        profiles,
        links,
    })
}

/// 把登录结果 + 用户选定的角色登记为一个皮肤站账号（同角色旧记录会被替换）。
#[tauri::command]
pub async fn yggdrasil_add_account(
    state: tauri::State<'_, AppState>,
    server_url: String,
    server_name: String,
    access_token: String,
    client_token: String,
    profile_id: String,
    profile_name: String,
) -> Result<Account, String> {
    if profile_id.is_empty() || profile_name.is_empty() {
        return Err("缺少角色信息".into());
    }
    let account = Account::Yggdrasil {
        uuid: profile_id,
        username: profile_name,
        created: now_secs(),
        access_token,
        client_token,
        server: normalize_root(&server_url),
        server_name: if server_name.trim().is_empty() {
            "自定义皮肤站".into()
        } else {
            server_name.trim().to_string()
        },
    };
    let mut list = load_accounts(state.inner());
    list.retain(|a| a.uuid() != account.uuid());
    list.push(account.clone());
    save_accounts(state.inner(), &list)?;
    Ok(account)
}

/// 校验令牌是否有效（`/authserver/validate` 返回 204 即有效）。
pub async fn validate(
    state: &AppState,
    root: &str,
    access_token: &str,
    client_token: &str,
) -> bool {
    match post_json(
        state,
        &format!("{root}/authserver/validate"),
        &json!({ "accessToken": access_token, "clientToken": client_token }),
    )
    .await
    {
        Ok((status, _)) => status == 204 || status == 200,
        Err(_) => false,
    }
}

/// 刷新令牌（rotate：返回新令牌，旧的立即失效）。
pub async fn refresh(
    state: &AppState,
    root: &str,
    access_token: &str,
    client_token: &str,
) -> Result<String, String> {
    let (status, body) = post_json(
        state,
        &format!("{root}/authserver/refresh"),
        &json!({ "accessToken": access_token, "clientToken": client_token }),
    )
    .await?;
    if status != 200 {
        return Err(server_error(status, &body, "令牌已失效，请重新登录皮肤站"));
    }
    body.get("accessToken")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "皮肤站未返回新令牌".into())
}

/// 启动前调用：令牌有效则原样返回；失效则尝试刷新并持久化新令牌。
/// 刷新也失败时返回错误，提示用户重新登录。
pub async fn ensure_token(state: &AppState, account_uuid: &str) -> Result<String, String> {
    let account = {
        let list = load_accounts(state);
        list.iter()
            .find(|a| a.uuid() == account_uuid)
            .cloned()
            .ok_or("账号不存在")?
    };
    let (root, access_token, client_token) = match &account {
        Account::Yggdrasil {
            server,
            access_token,
            client_token,
            ..
        } => (server.clone(), access_token.clone(), client_token.clone()),
        _ => return Err("该账号不是皮肤站账号".into()),
    };
    if validate(state, &root, &access_token, &client_token).await {
        return Ok(access_token);
    }
    let new_token = refresh(state, &root, &access_token, &client_token).await?;
    let mut list = load_accounts(state);
    if let Some(acc) = list.iter_mut().find(|a| a.uuid() == account_uuid) {
        if let Account::Yggdrasil { access_token, .. } = acc {
            *access_token = new_token.clone();
        }
    }
    save_accounts(state, &list)?;
    Ok(new_token)
}

/// 命令包装：供前端在启动前手动触发令牌保活。
#[tauri::command]
pub async fn yggdrasil_ensure_token(
    state: tauri::State<'_, AppState>,
    account_uuid: String,
) -> Result<String, String> {
    ensure_token(state.inner(), &account_uuid).await
}

/// 从 GitHub Releases 拿最新版 authlib-injector jar 的下载地址（兜底源）。
///
/// `api.github.com` 在国内基本不可直连，所以和 `terracotta` 一样逐个套加速前缀。
async fn github_latest_jar(probe: &reqwest::Client) -> Result<String, String> {
    const API: &str = "https://api.github.com/repos/yushijinhun/authlib-injector/releases/latest";
    let mut last_err = String::new();
    // 只试前 3 个（直连 + 两个最常用的加速前缀）：这一步是启动前的阻塞探测，
    // 把 6 个全试一遍在最坏情况下要多等一分多钟，得不偿失。
    for proxy in crate::terracotta::GITHUB_PROXIES.iter().take(3) {
        let url = crate::terracotta::proxied(API, proxy);
        let resp = match probe
            .get(&url)
            .header("User-Agent", "QookiX-Launcher")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{e}");
                continue;
            }
        };
        let resp = match resp.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{e}");
                continue;
            }
        };
        let rel: Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("响应解析失败: {e}");
                continue;
            }
        };
        for asset in rel.get("assets").and_then(|v| v.as_array()).into_iter().flatten() {
            let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.starts_with("authlib-injector") && name.ends_with(".jar") {
                if let Some(u) = asset.get("browser_download_url").and_then(|v| v.as_str()) {
                    return Ok(u.to_string());
                }
            }
        }
        last_err = "release 中未找到 jar 资源".to_string();
    }
    Err(format!("GitHub 请求失败: {last_err}"))
}

/// 从 `.../releases/download/v1.2.8/authlib-injector-1.2.8.jar` 里取出 `1.2.8`
fn version_from_jar_url(u: &str) -> Option<String> {
    let name = u.rsplit('/').next()?;
    let rest = name
        .strip_prefix("authlib-injector-")?
        .strip_suffix(".jar")?;
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

/// 确保本地存在 authlib-injector.jar，返回其路径。
///
/// jar 不在皮肤站 meta 里，从官方 artifact 接口（latest.json）获取：
/// 官方源在前，LittleSkin 镜像兜底（国内直连官方源常超时）。
/// 本地已有时按 sha256 决定复用还是重新下载；latest.json 拉不到
/// （如离线）但本地有旧 jar 时宽容复用。
pub async fn ensure_authlib_injector(
    state: &AppState,
    _api_root: &str,
) -> Result<PathBuf, String> {
    const OFFICIAL: &str = "https://authlib-injector.moe.yushi.moe";
    /// 官方站点（authlib-injector 的真实 artifact 域名，moe 版是它的镜像）
    const OFFICIAL_ALT: &str = "https://authlib-injector.yushi.moe";
    const MIRROR: &str = "https://download.littleskin.cn/moe/yushi/authlibinjector";
    /// 元数据全部拿不到时的兜底版本。GitHub 的 release 永久保留，
    /// 所以直接按这个版本号拼地址仍然能下到 jar（此时没有 sha256 可校验）。
    const FALLBACK_VERSION: &str = "1.2.8";

    let dest_dir = state.root.join("authlib-injector");
    let dest = dest_dir.join("authlib-injector.jar");

    // 本地已有 jar：零网络等待直接复用（更新检查只在 jar 缺失时才联网做）
    if dest.exists() && std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false) {
        return Ok(dest);
    }

    // 探测用 client：超时比业务 client 短（坏源尽快失败、别拖住启动），
    // 但必须沿用用户配置的代理——否则配了代理也照样直连，元数据全挂。
    let probe = {
        let s = state.settings();
        crate::settings::http_client_timed(
            &s.proxy_mode,
            s.proxy.as_deref(),
            std::time::Duration::from_secs(8),
            std::time::Duration::from_secs(20),
        )
    };

    let mut latest: Option<Value> = None;
    let mut last_err = String::new();
    for base in [OFFICIAL, OFFICIAL_ALT, MIRROR] {
        match crate::download::get_json(&probe, &format!("{base}/artifact/latest.json")).await {
            Ok(v) => {
                latest = Some(v);
                break;
            }
            Err(e) => last_err = e,
        }
    }
    let expected = latest
        .as_ref()
        .and_then(|v| v.pointer("/checksums/sha256"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase());

    // 候选下载地址（国内优先）。authlib-injector 官方 latest.json 的 `url` 指向
    // github.com，而旧的镜像兜底只会改写「官方域名」前缀的地址，对 github 地址
    // 完全不生效——结果国内直连不通时必然失败。这里改为按版本号自己拼 artifact
    // 路径：先 LittleSkin 镜像，再官方 artifact，再 GitHub Release，最后才用
    // latest.json 给的原始 url（并补上偶尔缺失的 `.jar` 点号）。
    let version = latest
        .as_ref()
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut candidates: Vec<String> = Vec::new();
    // ① LittleSkin 镜像的 artifact 路径（国内最快；若镜像不托管 jar 会很快 404）
    if let Some(v) = &version {
        candidates.push(format!("{MIRROR}/artifact/{v}/authlib-injector-{v}.jar"));
    }
    // ② GitHub Release：直连 + 项目既有的加速前缀（大陆直连 github 常被重置）
    if let Some(v) = &version {
        let gh = format!(
            "https://github.com/yushijinhun/authlib-injector/releases/download/v{v}/authlib-injector-{v}.jar"
        );
        for proxy in crate::terracotta::GITHUB_PROXIES {
            push_unique(&mut candidates, crate::terracotta::proxied(&gh, proxy));
        }
    }
    // ③ 官方 artifact 路径
    if let Some(v) = &version {
        push_unique(
            &mut candidates,
            format!("{OFFICIAL}/artifact/{v}/authlib-injector-{v}.jar"),
        );
    }
    // ④ latest.json 给的原始地址（补上偶尔缺失的 `.jar` 点号）
    if let Some(u) = latest
        .as_ref()
        .and_then(|l| l.get("url"))
        .and_then(|v| v.as_str())
    {
        push_unique(&mut candidates, normalize_jar_url(u));
    }
    if candidates.is_empty() {
        // 元数据三个源全挂：先问 GitHub API（走加速前缀），再退到内置兜底版本。
        match github_latest_jar(&probe).await {
            Ok(u) => {
                for proxy in crate::terracotta::GITHUB_PROXIES {
                    push_unique(&mut candidates, crate::terracotta::proxied(&u, proxy));
                }
                if let Some(v) = version_from_jar_url(&u) {
                    push_unique(
                        &mut candidates,
                        format!("{MIRROR}/artifact/{v}/authlib-injector-{v}.jar"),
                    );
                    push_unique(
                        &mut candidates,
                        format!("{OFFICIAL}/artifact/{v}/authlib-injector-{v}.jar"),
                    );
                }
            }
            Err(gh_err) => {
                // 连 API 都问不到：直接用内置版本号拼地址，仍比"直接失败"有用。
                eprintln!(
                    "[authlib] 版本信息获取失败({last_err} / {gh_err})，改用内置版本 {FALLBACK_VERSION}"
                );
                let v = FALLBACK_VERSION;
                push_unique(
                    &mut candidates,
                    format!("{MIRROR}/artifact/{v}/authlib-injector-{v}.jar"),
                );
                let gh = format!(
                    "https://github.com/yushijinhun/authlib-injector/releases/download/v{v}/authlib-injector-{v}.jar"
                );
                for proxy in crate::terracotta::GITHUB_PROXIES {
                    push_unique(&mut candidates, crate::terracotta::proxied(&gh, proxy));
                }
                push_unique(
                    &mut candidates,
                    format!("{OFFICIAL}/artifact/{v}/authlib-injector-{v}.jar"),
                );
            }
        }
    }

    async fn fetch(state: &AppState, url: &str) -> Result<Vec<u8>, String> {
        state
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    }
    // 逐个候选地址尝试：连不上 / 404 / 校验不过都换下一个，全部失败才报错。
    // 每个源单独限时，避免某个坏源（如被墙的 github）把启动卡上几分钟。
    let per_try = std::time::Duration::from_secs(12);
    let mut errs: Vec<String> = Vec::new();
    let mut downloaded: Option<Vec<u8>> = None;
    for url in &candidates {
        match tokio::time::timeout(per_try, fetch(state, url)).await {
            Ok(Ok(bytes)) => {
                if let Some(want) = &expected {
                    use sha2::Digest;
                    let actual = sha2::Sha256::digest(&bytes);
                    let hex: String = actual.iter().map(|b| format!("{b:02x}")).collect();
                    if !hex.eq_ignore_ascii_case(want) {
                        errs.push(format!("{url}: sha256 不匹配"));
                        continue;
                    }
                } else if bytes.len() < 4 || &bytes[0..2] != b"PK" {
                    // 没有 sha256 可校验时，至少确认拿到的是 zip（jar）而不是错误页
                    errs.push(format!("{url}: 返回内容不是 jar"));
                    continue;
                }
                downloaded = Some(bytes);
                break;
            }
            Ok(Err(e)) => errs.push(format!("{url}: {e}")),
            Err(_) => errs.push(format!("{url}: 超时")),
        }
    }

    let bytes = downloaded.ok_or_else(|| {
        format!(
            "下载 authlib-injector 失败（已尝试 {} 个地址）: {}。\
             可手动下载 authlib-injector.jar 放到 {} 后重试（该文件存在时会直接复用）。",
            candidates.len(),
            errs.join("；"),
            dest.display()
        )
    })?;

    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    std::fs::write(&dest, &bytes).map_err(|e| format!("写入 authlib-injector 失败: {e}"))?;
    Ok(dest)
}

/// latest.json 里的下载地址偶尔缺 `.jar` 前的点（`...-1.2.8jar`），补回来。
fn normalize_jar_url(u: &str) -> String {
    if u.ends_with(".jar") {
        return u.to_string();
    }
    match u.strip_suffix("jar") {
        Some(stem) => format!("{stem}.jar"),
        None => u.to_string(),
    }
}

/// 去重后追加一个候选地址。
fn push_unique(list: &mut Vec<String>, url: String) {
    if !list.iter().any(|existing| existing == &url) {
        list.push(url);
    }
}

fn load_accounts(state: &AppState) -> Vec<Account> {
    crate::accounts::load_accounts(state)
}

fn save_accounts(state: &AppState, list: &[Account]) -> Result<(), String> {
    crate::accounts::save_accounts(state, list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_root_trims_slash_and_space() {
        assert_eq!(
            normalize_root(" https://littleskin.cn/api/yggdrasil/ "),
            "https://littleskin.cn/api/yggdrasil"
        );
        assert_eq!(normalize_root(""), "");
        assert_eq!(normalize_root("https://skin.example.com/yggdrasil"), "https://skin.example.com/yggdrasil");
    }

    #[test]
    fn littleskin_official_root_survives_normalization() {
        assert_eq!(
            normalize_root("https://littleskin.cn/api/yggdrasil"),
            "https://littleskin.cn/api/yggdrasil"
        );
    }

    /// latest.json 偶尔给出缺点的地址（`...-1.2.8jar`），必须补回来
    #[test]
    fn normalize_jar_url_inserts_missing_dot() {
        let bad = "https://github.com/x/y/releases/download/v1.2.8/authlib-injector-1.2.8jar";
        assert_eq!(
            normalize_jar_url(bad),
            "https://github.com/x/y/releases/download/v1.2.8/authlib-injector-1.2.8.jar"
        );
        // 正常地址原样返回
        let good = "https://github.com/x/y/releases/download/v1.2.8/authlib-injector-1.2.8.jar";
        assert_eq!(normalize_jar_url(good), good);
        // 非 jar 结尾不动
        assert_eq!(normalize_jar_url("https://a/b/c.json"), "https://a/b/c.json");
    }

    #[test]
    fn version_from_jar_url_extracts_version() {
        assert_eq!(
            version_from_jar_url(
                "https://github.com/yushijinhun/authlib-injector/releases/download/v1.2.8/authlib-injector-1.2.8.jar"
            )
            .as_deref(),
            Some("1.2.8")
        );
        assert_eq!(version_from_jar_url("https://a/b/other.jar"), None);
        assert_eq!(version_from_jar_url("https://a/b/authlib-injector-.jar"), None);
    }

    #[test]
    fn push_unique_dedupes() {
        let mut v: Vec<String> = vec!["a".into()];
        push_unique(&mut v, "a".into());
        push_unique(&mut v, "b".into());
        assert_eq!(v, vec!["a".to_string(), "b".to_string()]);
    }
}
