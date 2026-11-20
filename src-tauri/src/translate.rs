//! 内容描述翻译：对接自建翻译服务（trans.zhayi.cc）。
//! 带本地磁盘缓存（`cache/translations.json`，TTL 7 天），命中缓存零配额消耗。

use crate::state::AppState;
use serde_json::{json, Value};
use std::collections::BTreeMap;
mod cache;
use cache::{CacheBatch, Purpose, Service};

const BASE: &str = "https://trans.zhayi.cc";
const LANG: &str = "zh";
/// 缓存有效期：7 天
const CACHE_TTL: u64 = 7 * 24 * 3600;
/// 服务端批量接口单次上限
const BATCH: usize = 5;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 批量翻译模组描述：先查本地缓存，未命中的按 5 个一组请求服务端。
///
/// 返回 `{ translations: {slug: 中文}, failed: [slug], rateLimited: bool }`；
/// 失败的 slug 不出现在 `translations` 里，前端回退显示原文。
/// 触发 429 时停止本轮剩余请求，避免继续白烧配额。
pub async fn translate_descriptions(
    state: &AppState,
    provider: &str,
    slugs: Vec<String>,
) -> Result<Value, String> {
    // 选择翻译服务：默认（自建服务）或自定义（用户自己的 OpenAI 兼容 AI API）
    let (custom_ready, api_base, api_key, api_model) = {
        let s = state.settings();
        (
            s.translate_provider == "custom",
            s.translate_api_base.trim().trim_end_matches('/').to_string(),
            s.translate_api_key.clone().unwrap_or_default(),
            s.translate_api_model.trim().to_string(),
        )
    };
    let use_custom = custom_ready && !api_base.is_empty() && !api_key.is_empty() && !api_model.is_empty();
    // 内置服务已支持 CurseForge 资源，两套服务覆盖相同的平台范围
    // 两套服务的翻译结果质量不同，缓存分开存放，来回切换互不覆盖
    let service = if use_custom { Service::custom(&api_base, &api_model) } else { Service::Builtin };
    let mut cache = CacheBatch::new(&state.root, service, provider, Purpose::Summary, now_secs());
    let mut translations = BTreeMap::new();
    let mut failed: Vec<String> = Vec::new();
    let mut queue: Vec<String> = Vec::new();

    for slug in slugs {
        let hit = cache.get(&slug, CACHE_TTL);
        match hit {
            Some(text) => {
                translations.insert(slug, json!(text));
            }
            None => queue.push(slug),
        }
    }

    let mut rate_limited = false;
    // ---- 自定义服务：先从 Modrinth 取英文原文，再逐条交给用户的 AI 翻译 ----
    if use_custom {
        while !queue.is_empty() {
            let slug = queue.remove(0);
            // 两个平台的 project_info 都返回统一的 description 字段
            // （Modrinth 用 project id/slug 查询，CurseForge 用数字 mod id）
            let info = match provider {
                "curseforge" => crate::curseforge::project_info(state, &slug).await,
                _ => crate::modrinth::project_info(state, &slug).await,
            };
            let desc = info
                .ok()
                .and_then(|p| {
                    p.get("description")
                        .and_then(|d| d.as_str())
                        .map(|s| s.trim().to_string())
                })
                .filter(|d| !d.is_empty());
            let Some(desc) = desc else {
                failed.push(slug);
                continue;
            };
            match chat_translate(state, &api_base, &api_key, &api_model, &desc).await {
                Ok(text) => {
                    cache.add(&slug, &text);
                    translations.insert(slug, json!(text));
                }
                Err(e) => {
                    // 鉴权失败 / 限流时继续请求没有意义，剩余全部记为失败
                    rate_limited = e.contains("429");
                    let fatal = e.contains("鉴权失败") || rate_limited;
                    failed.push(slug);
                    if fatal {
                        failed.extend(queue.drain(..));
                        break;
                    }
                }
            }
        }
        cache.commit();
        return Ok(json!({
            "translations": translations,
            "failed": failed,
            "rateLimited": rate_limited,
        }));
    }
    // ---- 默认服务：按 5 个一组请求自建接口 ----
    while !queue.is_empty() {
        let group: Vec<String> = queue.drain(..queue.len().min(BATCH)).collect();
        let body = json!({ "platform": provider, "lang": LANG, "mod_ids": group });
        let mut rb = state
            .client
            .post(format!("{BASE}/translate/mods"))
            .json(&body);
        if let Some(token) = crate::qookix::api::optional_access_token(&state).await {
            rb = rb.bearer_auth(token);
        }
        let resp = rb.send().await;

        let parsed = match resp {
            Ok(r) if r.status().as_u16() == 200 => r.json::<Value>().await.ok(),
            Ok(r) if r.status().as_u16() == 429 => {
                rate_limited = true;
                None
            }
            _ => None,
        };
        let Some(body) = parsed else {
            // 限流 / 服务不可达 / 响应异常：本组与剩余全部记为失败，停止请求
            failed.extend(group);
            failed.extend(queue.drain(..));
            break;
        };
        let results = body
            .get("results")
            .and_then(|r| r.as_object())
            .cloned()
            .unwrap_or_default();
        for slug in group {
            let text = results
                .get(&slug)
                .and_then(|e| e.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            if text.is_empty() {
                failed.push(slug);
                continue;
            }
            cache.add(&slug, &text);
            translations.insert(slug, json!(text));
        }
    }

    cache.commit();
    Ok(json!({
        "translations": translations,
        "failed": failed,
        "rateLimited": rate_limited,
    }))
}

/// 翻译资源正文（body）：详情弹窗右侧展示，译文优先、原文兜底。
///
/// - 内置服务与自定义 AI 均支持两平台（CurseForge 正文为 HTML）
/// - 返回 `{ body: 译文|null, bodyCached, original, supported, error? }`
pub async fn translate_body(
    state: &AppState,
    provider: &str,
    slug: &str,
    translate: bool,
) -> Result<Value, String> {
    // 拉原文（无译文时的兜底显示内容）
    let original = match provider {
        "curseforge" => crate::curseforge::project_description(state, slug).await,
        // Spigot：Spiget 的 description 是 base64 编码的 HTML，解码后即原文
        "spigot" => spigot_resource_description(state, slug).await,
        _ => {
            let info = crate::modrinth::project_info(state, slug).await?;
            Ok(info
                .get("body")
                .and_then(|b| b.as_str())
                .unwrap_or("")
                .to_string())
        }
    }
    .unwrap_or_default();

    let (custom_ready, api_base, api_key, api_model) = {
        let s = state.settings();
        (
            s.translate_provider == "custom",
            s.translate_api_base.trim().trim_end_matches('/').to_string(),
            s.translate_api_key.clone().unwrap_or_default(),
            s.translate_api_model.trim().to_string(),
        )
    };
    let use_custom = custom_ready && !api_base.is_empty() && !api_key.is_empty() && !api_model.is_empty();
    let service = if use_custom { Service::custom(&api_base, &api_model) } else { Service::Builtin };
    let mut cache = CacheBatch::new(&state.root, service, provider, Purpose::Body, now_secs());
    // 缓存优先：手动翻译过的正文再次打开详情时直接展示译文（translate=false 也命中）。
    // 用户不想要译文时点「显示原文」切换即可；清空缓存可彻底回到未翻译态。
    if let Some(hit) = cache.get(slug, CACHE_TTL) {
        return Ok(json!({ "body": hit, "bodyCached": true, "original": original, "supported": true }));
    }
    if original.trim().is_empty() {
        return Ok(json!({ "body": null, "bodyCached": false, "original": original, "supported": false }));
    }
    // translate=false 且无缓存：只返回原文，不调翻译服务
    if !translate {
        return Ok(json!({ "body": null, "bodyCached": false, "original": original, "supported": true }));
    }
    // 正文超长时截断（与服务端 maxBodyTranslateChars 对齐），避免请求被拒
    const MAX_BODY_CHARS: usize = 12000;
    let clipped: String = original.chars().take(MAX_BODY_CHARS).collect();

    // 自定义 AI：两平台通用
    if use_custom {
        return match chat_translate(state, &api_base, &api_key, &api_model, &clipped).await {
            Ok(text) => {
                cache.add(slug, &text);
                cache.commit();
                Ok(json!({ "body": text, "bodyCached": false, "original": original, "supported": true }))
            }
            Err(e) => Ok(json!({ "body": null, "bodyCached": false, "original": original, "supported": true, "error": e })),
        };
    }
    // 内置服务已支持 CurseForge 资源，platform 原样传给服务端
    // 正文翻译同步耗时 10~60s+，用独立长超时 client（全局 client 是 60s 会掐断）
    let slow = reqwest::Client::builder()
        .user_agent(concat!("QookiX-Launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(150))
        .build()
        .map_err(|e| e.to_string())?;
    let req = json!({ "platform": provider, "lang": LANG, "mod_id": slug, "include_body": true });
    let mut rb = slow.post(format!("{BASE}/translate/mod")).json(&req);
    if let Some(token) = crate::qookix::api::optional_access_token(&state).await {
        rb = rb.bearer_auth(token);
    }
    let resp = rb
        .send()
        .await
        .map_err(|e| format!("请求翻译服务失败: {e}"))?;
    let status = resp.status().as_u16();
    // 400 等错误回包可能是纯文本，先按文本读再尝试 JSON 解析
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let parsed: Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| json!({ "error": text.trim().chars().take(200).collect::<String>() }));
    if status != 200 {
        let msg = parsed.get("error").and_then(|v| v.as_str()).unwrap_or("翻译失败");
        return Err(format!("翻译服务返回 HTTP {status}: {msg}"));
    }
    match parsed
        .get("body")
        .and_then(|b| b.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(text) => {
            cache.add(slug, &text);
            cache.commit();
            Ok(json!({
                "body": text,
                "bodyCached": parsed.get("body_cached").and_then(|v| v.as_bool()).unwrap_or(false),
                "original": original,
                "supported": true,
            }))
        }
        None => {
            let err = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("服务未返回正文译文")
                .to_string();
            Ok(json!({ "body": null, "bodyCached": false, "original": original, "supported": true, "error": err }))
        }
    }
}

/// 模组名译文缓存有效期：名字基本不变，缓存久一点（30 天）
const NAME_CACHE_TTL: u64 = 30 * 24 * 3600;
/// 单次请求交给 AI 的名字个数
const NAME_BATCH: usize = 20;

/// 批量翻译模组名（英文 → 中文玩家社区常用称呼）。
///
/// 自建服务只提供「按平台 mod id 的描述译文」，不含名字，所以名字翻译走
/// 用户在设置里配置的 AI（OpenAI 兼容）服务；未配置时返回 `unsupported: true`，
/// 前端保持显示原名。结果带磁盘缓存（30 天）。
pub async fn translate_mod_names(state: &AppState, names: Vec<String>) -> Result<Value, String> {
    let (api_base, api_key, api_model) = {
        let s = state.settings();
        (
            s.translate_api_base.trim().trim_end_matches('/').to_string(),
            s.translate_api_key.clone().unwrap_or_default(),
            s.translate_api_model.trim().to_string(),
        )
    };
    if api_base.is_empty() || api_key.is_empty() || api_model.is_empty() {
        return Ok(json!({ "translations": {}, "unsupported": true }));
    }
    let service = Service::custom(&api_base, &api_model);
    let mut cache = CacheBatch::new(&state.root, service, "*", Purpose::Name, now_secs());
    let mut translations: BTreeMap<String, Value> = BTreeMap::new();
    let mut queue: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for name in names {
        let name = name.trim().to_string();
        if name.is_empty() || !seen.insert(name.to_lowercase()) {
            continue;
        }
        let hit = cache.get(&name.to_lowercase(), NAME_CACHE_TTL);
        match hit {
            Some(t) => {
                translations.insert(name, json!(t));
            }
            None => queue.push(name),
        }
    }

    let mut error: Option<String> = None;
    while !queue.is_empty() {
        let group: Vec<String> = queue.drain(..queue.len().min(NAME_BATCH)).collect();
        match chat_translate_names(state, &api_base, &api_key, &api_model, &group).await {
            Ok(map) => {
                for (orig, zh) in map {
                    cache.add(&orig.trim().to_lowercase(), &zh);
                    translations.insert(orig, json!(zh));
                }
            }
            Err(e) => {
                // 鉴权失败 / 限流时继续请求没意义
                error = Some(e);
                break;
            }
        }
    }
    cache.commit();
    Ok(json!({ "translations": translations, "unsupported": false, "error": error }))
}

/// 让 AI 把一批模组名翻成中文，返回 `{原名: 译名}`。
async fn chat_translate_names(
    state: &AppState,
    base: &str,
    key: &str,
    model: &str,
    names: &[String],
) -> Result<BTreeMap<String, String>, String> {
    // 用 JSON 数组传输入：模型不会把列表符号（- 等）当成名字的一部分
    let list = serde_json::to_string(names).unwrap_or_else(|_| "[]".into());
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "temperature": 0.1,
        "messages": [
            {
                "role": "system",
                "content": "你是 Minecraft 模组名称翻译器。用户会给出一个 JSON 数组，元素是模组英文名。\
请为每个名字给出中文玩家社区里最常用的称呼：能意译的译成中文（如 Just Enough Items → 物品管理器，\
Create → 机械动力），纯缩写、品牌名或没有通用译名的保持原样（如 Sodium、JEI）。\
若某个名字在社区里就是直接使用英文，则原样保留。\
只输出一个 JSON 对象：键必须与输入数组中的元素逐字符一致，值为译文；\
不要输出思考过程、解释或 Markdown 代码块标记。"
            },
            { "role": "user", "content": format!("请翻译这些模组名称：{list}") }
        ],
    });
    let resp = state
        .client
        .post(url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求翻译服务失败: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!(
            "翻译服务返回 HTTP {status}: {}",
            text.trim().chars().take(200).collect::<String>()
        ));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|e| format!("解析翻译响应失败: {e}"))?;
    let content = parsed
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    parse_name_map(&content).ok_or_else(|| "AI 未返回可解析的 JSON".to_string())
}

/// 从模型输出里抠出 JSON 对象。
/// 容忍：推理模型的 ` thinking…` 思考过程、```json 代码块包裹、前后多余文字。
fn parse_name_map(content: &str) -> Option<BTreeMap<String, String>> {
    // 只取最后一次思考结束后的正文（思考里常出现示例 JSON，会干扰截取）
    let body = match content.rfind("") {
        Some(i) => &content[i + "".len()..],
        None => content,
    };
    let body = if body.trim().is_empty() { content } else { body };
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    if end <= start {
        return None;
    }
    let raw: BTreeMap<String, String> = serde_json::from_str(&body[start..=end]).ok()?;
    // 兜底清洗：模型偶尔把列表符号或空白带进键名
    Some(
        raw.into_iter()
            .map(|(k, v)| {
                let clean = k
                    .trim()
                    .trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_whitespace())
                    .trim()
                    .to_string();
                (clean, v.trim().to_string())
            })
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect(),
    )
}

/// 反馈某条翻译已过期。返回服务端 status；`updated` 时清掉本地缓存，
/// 下次请求即可拿到重新翻译的结果。
pub async fn report_stale(state: &AppState, provider: &str, slug: &str) -> Result<String, String> {
    let body = json!({ "platform": provider, "mod_id": slug, "lang": LANG });
    let resp = state
        .client
        .post(format!("{BASE}/feedback/stale"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let v: Value = resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
    let status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("received")
        .to_string();
    if status == "updated" {
        if let Err(err) = cache::invalidate_builtin(&state.root, provider, slug) {
            crate::util::log_line(&format!("[translate] cache invalidation failed: {err}"));
        }
    }
    Ok(status)
}

/// 反馈翻译质量问题（issue_type + 可选的建议翻译与补充说明）。
/// 服务端限流 5 次/天/IP，超限静默返回 received，客户端无需特殊处理。
pub async fn report_quality(
    state: &AppState,
    provider: &str,
    slug: &str,
    issue_type: &str,
    suggestion: Option<String>,
    comment: Option<String>,
) -> Result<String, String> {
    match issue_type {
        "wrong_translation" | "unnatural" | "missing" | "other" => {}
        _ => return Err("未知的问题类型".into()),
    }
    let mut body = json!({
        "platform": provider,
        "mod_id": slug,
        "lang": LANG,
        "issue_type": issue_type,
    });
    if let Some(s) = suggestion.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        body["user_suggestion"] = json!(s);
    }
    if let Some(c) = comment.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        body["user_comment"] = json!(c);
    }
    let resp = state
        .client
        .post(format!("{BASE}/feedback/quality"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let v: Value = resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
    Ok(v.get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("received")
        .to_string())
}

/// Clear translations for one service or the entire cache.
pub fn clear_cache(state: &AppState, service: Option<&str>) -> Result<u64, String> {
    cache::clear(&state.root, service)
}

/// 从 Spiget 拉取 SpigotMC 资源的原始描述（base64 编码的 HTML），解码返回。
/// 元数据接口公开无需鉴权，客户端实测国内可直连。
async fn spigot_resource_description(
    state: &AppState,
    resource_id: &str,
) -> Result<String, String> {
    let resp = state
        .client
        .get(format!("https://api.spiget.org/v2/resources/{resource_id}"))
        .send()
        .await
        .map_err(|e| format!("请求 Spiget 失败: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status == 404 {
        return Err("Spiget 上没有该资源（可能为付费资源，被 Spigot 拦截查询）".into());
    }
    if status != 200 {
        return Err(format!("Spiget 返回 HTTP {status}"));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("Spiget 响应解析失败: {e}"))?;
    let encoded = v
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| format!("描述 base64 解码失败: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// 调用用户的 OpenAI 兼容接口翻译一段文本。
pub async fn chat_translate(
    state: &AppState,
    base: &str,
    key: &str,
    model: &str,
    text: &str,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", base.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "temperature": 0.2,
        "messages": [
            {
                "role": "system",
                "content": "你是 Minecraft 模组描述的翻译器。把用户提供的英文模组描述准确翻译成简体中文：保留模组名、专有名词与技术术语；只输出译文本身，不要任何解释、引号或前后缀。"
            },
            { "role": "user", "content": text }
        ],
    });
    let resp = state
        .client
        .post(url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err(format!("鉴权失败（HTTP {status}），请检查 API Key"));
    }
    if status == 429 {
        return Err("自定义服务限流（429）".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("自定义翻译服务返回 HTTP {status}"));
    }
    let v: Value = resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?;
    let content = v
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty());
    content.ok_or_else(|| "自定义服务未返回翻译内容".into())
}
