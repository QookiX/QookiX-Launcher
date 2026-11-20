use crate::{accounts, image_files};
use crate::models::*;
use crate::state::AppState;
use tauri::State;

// Skins
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, Clone)]
pub struct SkinEntry {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub path: String,
    pub size: u64,
    pub modified: u64,
}

/// 内容指纹：SHA256 前 8 位 hex，用作皮肤隐式 id，使同名不同内容的皮肤互不覆盖。
fn skin_id_from_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hash = hasher.finalize();
    let mut s = String::with_capacity(8);
    for b in &hash[..4] {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// 从文件 stem 解析显示名：去掉末尾 `.{8位hex}` id 段，无则原样返回（兼容旧文件）。
fn display_name_from_stem(stem: &str) -> String {
    let parts: Vec<&str> = stem.rsplitn(2, '.').collect();
    if parts.len() == 2
        && parts[0].len() == 8
        && parts[0].chars().all(|c| c.is_ascii_hexdigit())
    {
        parts[1].to_string()
    } else {
        stem.to_string()
    }
}

/// List all `.png` skins in the `skins` directory of the data root.
#[tauri::command]
pub fn list_skins(state: State<AppState>) -> Result<Vec<SkinEntry>, String> {
    let dir = state.root.join("skins");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建皮肤目录失败: {e}"))?;
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("读取皮肤目录失败: {e}"))?;
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("png")) != Some(true) {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        let id = stem.clone();
        let name = display_name_from_stem(&stem);
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        out.push(SkinEntry {
            id,
            name,
            filename,
            path: path.to_string_lossy().to_string(),
            size: meta.len(),
            modified,
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

/// Read a skin file (by filename in the skins dir, or absolute path from a
/// native file-picker) as a data URL.
#[tauri::command]
pub fn read_skin_data_url(state: State<AppState>, filename: String) -> Result<String, String> {
    let is_abs = std::path::Path::new(&filename).is_absolute();
    let path = if is_abs {
        std::path::PathBuf::from(&filename)
    } else {
        if !crate::fsutil::is_safe_filename(&filename) {
            return Err("非法文件名".into());
        }
        state.root.join("skins").join(&filename)
    };
    let bytes = image_files::read_png(&path)?;
    Ok(image_files::data_url(&bytes, "image/png"))
}

/// Save a skin PNG (base64 without data: prefix or with it) to the skins directory.
#[tauri::command]
pub fn save_skin_from_data(state: State<AppState>, name: String, data: String) -> Result<SkinEntry, String> {
    let bytes = image_files::decode_png(&data)?;
    save_skin(&state, &name, &bytes)
}

fn save_skin(state: &AppState, name: &str, bytes: &[u8]) -> Result<SkinEntry, String> {
    let safe_name: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    if safe_name.is_empty() {
        return Err("皮肤名称不能为空".into());
    }
    let id = skin_id_from_bytes(bytes);
    let dir = state.root.join("skins");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建皮肤目录失败: {e}"))?;
    let path = dir.join(format!("{}.{}.png", safe_name, id));
    crate::fsutil::atomic_write(&path, bytes).map_err(|e| format!("写入皮肤文件失败: {e}"))?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("读取皮肤元信息失败: {e}"))?;
    Ok(SkinEntry {
        id,
        name: safe_name,
        filename: path.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string(),
        path: path.to_string_lossy().to_string(),
        size: meta.len(),
        modified: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    })
}

/// Download a skin PNG from a URL and save it to the skins directory.
#[tauri::command]
pub async fn download_skin_from_url(
    state: State<'_, AppState>,
    name: String,
    url: String,
) -> Result<SkinEntry, String> {
    let (bytes, _) = image_files::download(&state.client, &url, true).await?;
    save_skin(&state, &name, &bytes)
}

/// Delete a skin file by filename in the skins directory.
#[tauri::command]
pub fn delete_skin(state: State<AppState>, filename: String) -> Result<(), String> {
    if !crate::fsutil::is_safe_filename(&filename) {
        return Err("非法文件名".into());
    }
    let path = state.root.join("skins").join(&filename);
    if !path.exists() {
        return Err("皮肤文件不存在".into());
    }
    std::fs::remove_file(&path).map_err(|e| format!("删除皮肤失败: {e}"))
}

/// Fetch a player's skin by Minecraft username via Mojang API.
/// Returns the skin PNG as a data URL plus model type ("classic" or "slim").
#[derive(serde::Serialize)]
pub struct PlayerSkinResult {
    pub data_url: String,
    pub model: String,
    pub cape_data_url: Option<String>,
}

#[tauri::command]
pub async fn fetch_image_data_url(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let (bytes, mime) = image_files::download(&state.client, &url, false).await?;
    Ok(image_files::data_url(&bytes, mime))
}

#[tauri::command]
pub async fn fetch_player_skin(state: State<'_, AppState>, username: String) -> Result<PlayerSkinResult, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let trimmed = username.trim();
    if trimmed.is_empty() {
        return Err("玩家名不能为空".into());
    }
    let profile_url = format!("https://api.mojang.com/users/profiles/minecraft/{}", trimmed);
    let profile: serde_json::Value = state
        .client
        .get(&profile_url)
        .send()
        .await
        .map_err(|e| format!("查询玩家失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析玩家信息失败: {e}"))?;
    let uuid = profile
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("未找到该玩家（可能不存在或为离线账号）")?
        .to_string();
    let session_url = format!("https://sessionserver.mojang.com/session/minecraft/profile/{}", uuid);
    let session: serde_json::Value = state
        .client
        .get(&session_url)
        .send()
        .await
        .map_err(|e| format!("获取会话信息失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("解析会话信息失败: {e}"))?;
    let props = session.get("properties").and_then(|v| v.as_array()).ok_or("玩家无皮肤信息")?;
    let mut skin_url: Option<String> = None;
    let mut skin_model = "classic".to_string();
    let mut cape_url: Option<String> = None;
    for p in props {
        if p.get("name").and_then(|v| v.as_str()) == Some("textures") {
            let value = p.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let decoded = STANDARD.decode(value).map_err(|e| format!("解码 textures 失败: {e}"))?;
            let json_str = String::from_utf8(decoded).map_err(|e| format!("textures 不是 UTF-8: {e}"))?;
            let tex: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| format!("解析 textures JSON 失败: {e}"))?;
            if let Some(url) = tex
                .pointer("/textures/SKIN/url")
                .and_then(|v| v.as_str())
            {
                skin_url = Some(url.to_string());
            }
            if let Some(model) = tex
                .pointer("/textures/SKIN/metadata/model")
                .and_then(|v| v.as_str())
            {
                skin_model = model.to_string();
            }
            if let Some(url) = tex
                .pointer("/textures/CAPE/url")
                .and_then(|v| v.as_str())
            {
                cape_url = Some(url.to_string());
            }
        }
    }
    let skin_url = skin_url.ok_or("该玩家未设置自定义皮肤")?;
    let (bytes, mime) = image_files::download(&state.client, &skin_url, true).await?;
    let cape_data_url = match cape_url {
        Some(url) => image_files::download(&state.client, &url, true).await.ok()
            .map(|(data, mime)| image_files::data_url(&data, mime)),
        None => None,
    };

    Ok(PlayerSkinResult {
        data_url: image_files::data_url(&bytes, mime),
        model: skin_model,
        cape_data_url,
    })
}

/// Fetch all capes owned by a Microsoft account via Mojang API.
#[derive(serde::Serialize)]
pub struct CapeInfo {
    pub id: String,
    pub name: String,
    pub data_url: String,
    pub active: bool,
}

#[tauri::command]
pub async fn fetch_player_capes(
    state: State<'_, AppState>,
    account_uuid: String,
) -> Result<Vec<CapeInfo>, String> {
    let accounts = accounts::load_accounts(&state);
    let account = accounts
        .iter()
        .find(|a| a.uuid() == &account_uuid)
        .ok_or("账号不存在")?
        .clone();
    if !account.is_microsoft() {
        return Ok(Vec::new());
    }
    let account = accounts::refresh_microsoft(&state, &account).await?;
    let mc_token = match &account {
        Account::Microsoft {
            msa_access_token, ..
        } => msa_access_token.clone(),
        _ => return Err("账号类型错误".into()),
    };
    let resp = state
        .client
        .get("https://api.minecraftservices.com/minecraft/profile")
        .bearer_auth(&mc_token)
        .send()
        .await
        .map_err(|e| format!("获取披风列表失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("获取披风列表失败 (HTTP {status}): {body}"));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析披风列表失败: {e}"))?;
    let raw_capes = json
        .get("capes")
        .and_then(|v| v.as_array())
        .ok_or("披风列表格式异常")?;
    let mut result = Vec::new();
    for c in raw_capes {
        let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = c
            .get("alias")
            .and_then(|v| v.as_str())
            .unwrap_or("未命名披风")
            .to_string();
        let active = c
            .get("state")
            .and_then(|v| v.as_str())
            .map(|s| s.eq_ignore_ascii_case("ACTIVE"))
            .unwrap_or(false);
        let url = match c.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => continue,
        };
        let Ok((bytes, mime)) = image_files::download(&state.client, &url, true).await else { continue };
        result.push(CapeInfo {
            id,
            name,
            active,
            data_url: image_files::data_url(&bytes, mime),
        });
    }
    Ok(result)
}

/// Apply a cape to a Microsoft account. `cape_id` = None hides the cape.
#[tauri::command]
pub async fn apply_cape_to_account(
    state: State<'_, AppState>,
    account_uuid: String,
    cape_id: Option<String>,
) -> Result<(), String> {
    let accounts = accounts::load_accounts(&state);
    let account = accounts
        .iter()
        .find(|a| a.uuid() == &account_uuid)
        .ok_or("账号不存在")?
        .clone();
    if !account.is_microsoft() {
        return Err("离线账号无法应用披风".into());
    }
    let account = accounts::refresh_microsoft(&state, &account).await?;
    let mc_token = match &account {
        Account::Microsoft {
            msa_access_token, ..
        } => msa_access_token.clone(),
        _ => return Err("账号类型错误".into()),
    };
    let resp = if let Some(cid) = cape_id {
        state
            .client
            .put("https://api.minecraftservices.com/minecraft/profile/capes/active")
            .bearer_auth(&mc_token)
            .header("Content-Type", "application/json")
            .body(serde_json::json!({ "capeId": cid }).to_string())
            .send()
            .await
            .map_err(|e| format!("应用披风失败: {e}"))?
    } else {
        state
            .client
            .delete("https://api.minecraftservices.com/minecraft/profile/capes/active")
            .bearer_auth(&mc_token)
            .send()
            .await
            .map_err(|e| format!("隐藏披风失败: {e}"))?
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("应用披风失败 (HTTP {status}): {body}"));
    }
    Ok(())
}

/// Upload a skin PNG to the player's Mojang account.
/// `skin_data` is a base64 string or a `data:image/png;base64,...` URL.
/// `variant` is `"classic"` (default arms) or `"slim"`.
#[tauri::command]
pub async fn apply_skin_to_account(
    state: State<'_, AppState>,
    account_uuid: String,
    skin_data: String,
    variant: String,
) -> Result<(), String> {
    let bytes = image_files::decode_png(&skin_data)?;
    let accounts = accounts::load_accounts(&state);
    let account = accounts
        .iter()
        .find(|a| a.uuid() == &account_uuid)
        .ok_or("账号不存在")?
        .clone();
    if !account.is_microsoft() {
        return Err("离线账号无法上传皮肤，仅支持正版账号".into());
    }
    let account = accounts::refresh_microsoft(&state, &account).await?;
    let mc_token = match &account {
        Account::Microsoft {
            msa_access_token, ..
        } => msa_access_token.clone(),
        _ => return Err("账号类型错误".into()),
    };
    let v = if variant == "slim" { "slim" } else { "classic" };
    let file_part = reqwest::multipart::Part::bytes(bytes)
        .file_name("skin.png")
        .mime_str("image/png")
        .map_err(|e| format!("构造上传数据失败: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .text("variant", v.to_string())
        .part("file", file_part);
    let resp = state
        .client
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(&mc_token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("上传皮肤失败: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("上传皮肤失败 (HTTP {status}): {body}"));
    }
    Ok(())
}

/// Save the offline skin PNG. At launch time `launch::build_skin_resourcepack`
/// turns it into `resourcepacks/QookiX_Skin.zip` and enables that pack in
/// `options.txt`; `client.jar` is never modified.
/// The skin variant ("slim"/"classic") is persisted alongside in a JSON meta
/// file so it survives any frontend cache (localStorage) clears.
#[tauri::command]
pub fn apply_skin_offline(
    state: State<AppState>,
    skin_data: String,
    variant: String,
    uuid: String,
) -> Result<(), String> {
    crate::fsutil::validate_id(&uuid, "账号")?;
    let bytes = image_files::decode_png(&skin_data)?;

    let skin_dir = state.root.join("skins").join("offline");
    std::fs::create_dir_all(&skin_dir).map_err(|e| format!("创建皮肤目录失败: {e}"))?;
    crate::fsutil::atomic_write(&skin_dir.join(format!("{uuid}.png")), &bytes)
        .map_err(|e| format!("保存皮肤失败: {e}"))?;

    let variant = if variant == "slim" { "slim" } else { "classic" };
    let meta = serde_json::json!({ "variant": variant });
    let meta_str = serde_json::to_string_pretty(&meta)
        .unwrap_or_else(|_| r#"{"variant":"classic"}"#.to_string());
    std::fs::write(skin_dir.join(format!("{uuid}.json")), meta_str)
        .map_err(|e| format!("保存皮肤变体失败: {e}"))?;
    Ok(())
}

/// Read back a saved offline skin (PNG as a base64 data URL) plus its variant.
/// Returns `null` when no skin has been saved for that uuid.
#[tauri::command]
pub fn get_offline_skin(
    state: State<AppState>,
    uuid: String,
) -> Result<Option<serde_json::Value>, String> {
    crate::fsutil::validate_id(&uuid, "账号")?;

    let skin_dir = state.root.join("skins").join("offline");
    let png_path = skin_dir.join(format!("{uuid}.png"));
    if !png_path.is_file() {
        return Ok(None);
    }
    let bytes = image_files::read_png(&png_path)?;
    let src = image_files::data_url(&bytes, "image/png");

    // `None` when no meta file exists yet → frontend falls back to auto-detection.
    let variant = std::fs::read_to_string(skin_dir.join(format!("{uuid}.json")))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("variant").and_then(|s| s.as_str()).map(String::from))
        .filter(|v| v == "slim" || v == "classic");

    Ok(Some(serde_json::json!({ "src": src, "variant": variant })))
}

