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
    // 规范里 `/authserver/validate` 成功时返回 204 + 空响应体。以前这里直接
    // `resp.json()`，空 body 会报「解析失败」，validate 于是被误判成「令牌无效」，
    // 每次启动都白跑一次 refresh；而 refresh 会让旧令牌 rotate 失效，站点一旦
    // 不支持刷新（或有多端同时在用同一账号）就直接 403 Invalid token，启动失败。
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取皮肤站响应失败: {e}"))?;
    if text.trim().is_empty() {
        return Ok((status, Value::Null));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "皮肤站返回了非 JSON 响应（HTTP {status}）：{}",
            text.chars().take(120).collect::<String>()
        )
    })?;
    Ok((status, v))
}

/// validate / refresh 的请求体。`clientToken` 在 Yggdrasil 规范里是可选项，
/// 本地没有时就不要发空字符串——个别站点会把空值当成「与令牌不匹配」而 403。
fn token_body(access_token: &str, client_token: &str) -> Value {
    let mut body = json!({ "accessToken": access_token });
    if !client_token.trim().is_empty() {
        body["clientToken"] = json!(client_token);
    }
    body
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
    // 跳过 CDN 缓存（站点前面挂着 EdgeOne 之类的缓存，换肤后立刻重拉可能拿到旧图）
    cache_bust: Option<bool>,
) -> Result<Option<Value>, String> {
    let root = normalize_root(&server);
    let pid = profile_id.trim().to_string();
    if root.is_empty() || pid.is_empty() {
        return Ok(None);
    }
    let mut url = format!("{root}/sessionserver/session/minecraft/profile/{pid}");
    if cache_bust.unwrap_or(false) {
        // 时间戳放在 query 上：既绕开按 URL 缓存的 CDN，站点自身也会忽略这个参数
        url.push_str(&format!("?t={}", now_secs()));
    }
    let resp = state
        .client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("访问皮肤站 sessionserver 失败: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // 该角色没有纹理数据
    }
    // authlib-injector 约定：profile 不存在时返回 204 且响应体为空。
    // 不单独处理的话下面的 `resp.json()` 会因空 body 报错，让「没皮肤」和
    // 「站不可达」混成同一条错误，前端只能一律显示默认头像。
    if resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
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
    // 以服务端回填的 clientToken 为准：部分站点（如拾木空间）会生成自己的
    // clientToken 并要求后续 refresh 与之一致，继续用本地生成的会一直 403。
    let client_token = body
        .get("clientToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(client_token);
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
    crate::accounts::save_account(state.inner(), &account)?;
    Ok(account)
}

/// `/authserver/validate` 的三种结论。
enum TokenState {
    /// 令牌有效：规范返回 204（空响应体），个别实现返回 200 + 用户资料
    Valid,
    /// 站点明确判定令牌无效（401 / 403）
    Invalid,
    /// 网络异常或意外状态码：判断不了，**不能**据此刷新令牌
    Unknown(String),
}

/// 校验令牌是否有效（`/authserver/validate` 返回 204 即有效）。
async fn validate(
    state: &AppState,
    root: &str,
    access_token: &str,
    client_token: &str,
) -> TokenState {
    match post_json(
        state,
        &format!("{root}/authserver/validate"),
        &token_body(access_token, client_token),
    )
    .await
    {
        Ok((200 | 204, _)) => TokenState::Valid,
        Ok((401 | 403, _)) => TokenState::Invalid,
        Ok((status, _)) => TokenState::Unknown(format!("HTTP {status}")),
        Err(e) => TokenState::Unknown(e),
    }
}

/// 刷新结果：新令牌，以及站点可能回填的新 clientToken。
struct RefreshedToken {
    access_token: String,
    client_token: Option<String>,
}

/// 刷新令牌（rotate：返回新令牌，旧的立即失效）。
async fn refresh(
    state: &AppState,
    root: &str,
    access_token: &str,
    client_token: &str,
) -> Result<RefreshedToken, String> {
    let (status, body) = post_json(
        state,
        &format!("{root}/authserver/refresh"),
        &token_body(access_token, client_token),
    )
    .await?;
    if status != 200 {
        // 401/403 基本就是令牌已失效（expired / rotated / 站点清理过），
        // 刷新救不回来，直接把「该做什么」告诉用户，别只甩英文 errorMessage。
        if status == 401 || status == 403 {
            let detail = body
                .get("errorMessage")
                .and_then(|m| m.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("令牌无效");
            return Err(format!(
                "皮肤站登录状态已失效（{detail}），请重新登录该账号后再启动"
            ));
        }
        return Err(server_error(status, &body, "刷新令牌失败，请重新登录皮肤站"));
    }
    let access_token = body
        .get("accessToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or("皮肤站未返回新令牌")?;
    let client_token = body
        .get("clientToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(RefreshedToken {
        access_token,
        client_token,
    })
}

/// 启动前校验令牌，失效时尝试刷新并持久化。按完整账号匹配皮肤站和角色。
pub async fn ensure_token_for(state: &AppState, account: &Account) -> Result<String, String> {
    let initial = crate::accounts::snapshot_for_refresh(state, account)?;
    let lock = crate::accounts::refresh_lock(state, account.uuid());
    let _guard = lock.lock().await;
    let (mut current, version) = crate::accounts::current_for_refresh(state, account.uuid(), &initial)?;
    let (root, access_token, client_token) = match &mut current {
        Account::Yggdrasil {
            server,
            access_token,
            client_token,
            ..
        } => (server, access_token, client_token),
        _ => return Err("该账号不是皮肤站账号".into()),
    };
    match validate(state, root, access_token, client_token).await {
        TokenState::Valid => {
            crate::accounts::current_for_refresh(state, account.uuid(), &version)?;
            return Ok(access_token.clone());
        }
        TokenState::Invalid => {}
        TokenState::Unknown(reason) => {
            // 校验接口本身不可用（站点 5xx、网络抖动）时不要贸然刷新：
            // refresh 会让旧令牌 rotate 失效，反而把本来能用的令牌弄坏。
            // 直接放行，游戏内 authlib-injector 还会再校验一次。
            eprintln!("[yggdrasil] 无法确认令牌状态（{reason}），沿用现有令牌启动");
            crate::accounts::current_for_refresh(state, account.uuid(), &version)?;
            return Ok(access_token.clone());
        }
    }
    let refreshed = refresh(state, root, access_token, client_token).await?;
    access_token.clone_from(&refreshed.access_token);
    if let Some(new_client) = refreshed.client_token {
        *client_token = new_client;
    }
    crate::db::update_account(&state.db(), &current, &version)?;
    Ok(refreshed.access_token)
}

// Update both values from the upstream release's published asset digest.
const INJECTOR_VERSION: &str = "1.2.8";
const INJECTOR_SHA256: &str = "9c7f4343e6c82034958ffb48c14a2cb0c85928be7283103ce17da00c6d5a7b10";
const MAX_INJECTOR_BYTES: usize = 4 * 1024 * 1024;

fn verify_injector(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    if bytes.len() > MAX_INJECTOR_BYTES || format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err("authlib-injector SHA-256 校验失败".into());
    }
    Ok(())
}

async fn verified_local_injector(path: &std::path::Path) -> bool {
    use tokio::io::AsyncReadExt;
    let Ok(metadata) = tokio::fs::symlink_metadata(path).await else { return false };
    if !metadata.is_file() || metadata.len() > MAX_INJECTOR_BYTES as u64 {
        return false;
    }
    let Ok(file) = tokio::fs::File::open(path).await else { return false };
    let mut bytes = Vec::new();
    file.take(MAX_INJECTOR_BYTES as u64 + 1).read_to_end(&mut bytes).await.is_ok()
        && verify_injector(&bytes, INJECTOR_SHA256).is_ok()
}

/// Every launch verifies the installed agent against the upstream release digest.
pub async fn ensure_authlib_injector(state: &AppState, _api_root: &str) -> Result<PathBuf, String> {
    let dest_dir = state.root.join("authlib-injector");
    let dest = dest_dir.join("authlib-injector.jar");
    let lock = state.jobs.download_lock(dest.clone());
    let _install = lock.lock().await;
    if verified_local_injector(&dest).await {
        return Ok(dest);
    }

    let file_name = format!("authlib-injector-{INJECTOR_VERSION}.jar");
    let upstream = format!(
        "https://github.com/yushijinhun/authlib-injector/releases/download/v{INJECTOR_VERSION}/{file_name}"
    );
    let official_artifact = format!("artifact/{INJECTOR_VERSION}/{file_name}");
    let candidates = [
        format!("https://download.littleskin.cn/moe/yushi/authlibinjector/{official_artifact}"),
        format!("https://authlib-injector.yushi.moe/{official_artifact}"),
        upstream.clone(),
    ];
    let candidates = candidates.into_iter().chain(
        crate::terracotta::GITHUB_PROXIES.iter().filter(|proxy| !proxy.is_empty())
            .map(|proxy| crate::terracotta::proxied(&upstream, proxy)),
    );

    let mut errors = Vec::new();
    for url in candidates {
        let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
            let response = state.client.get(url).send().await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|err| err.to_string())?;
            let bytes = crate::util::read_response_limited(response, MAX_INJECTOR_BYTES).await?;
            verify_injector(&bytes, INJECTOR_SHA256)?;
            Ok::<_, String>(bytes)
        }).await;
        let bytes = match result {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(err)) => { errors.push(err); continue; }
            Err(_) => { errors.push("下载超时".into()); continue; }
        };
        std::fs::create_dir_all(&dest_dir).map_err(|err| format!("创建 authlib-injector 目录失败: {err}"))?;
        crate::fsutil::atomic_write(&dest, &bytes).map_err(|err| format!("写入 authlib-injector 失败: {err}"))?;
        return Ok(dest);
    }

    Err(format!(
        "下载并校验 authlib-injector 失败（{}）。可从 {} 获取版本 {}，确认 SHA-256 为 {} 后放入 {}。",
        errors.join("；"), upstream, INJECTOR_VERSION, INJECTOR_SHA256, dest.display(),
    ))
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

    #[test]
    fn injector_requires_exact_bytes_even_for_a_jar_header() {
        use sha2::{Digest, Sha256};
        let bytes = b"PK\x03\x04 sample jar";
        let expected = format!("{:x}", Sha256::digest(bytes));
        assert!(verify_injector(bytes, &expected).is_ok());
        assert!(verify_injector(b"PK\x03\x04 corrupted", &expected).is_err());
        assert!(verify_injector(bytes, "").is_err());
    }

    /// clientToken 为空时不能发空字符串：部分站点会当成「与令牌不匹配」直接 403
    #[test]
    fn token_body_omits_blank_client_token() {
        let body = token_body("at", "");
        assert_eq!(body.get("accessToken").and_then(|v| v.as_str()), Some("at"));
        assert!(body.get("clientToken").is_none());

        let body = token_body("at", "ct");
        assert_eq!(body.get("clientToken").and_then(|v| v.as_str()), Some("ct"));
    }
}
