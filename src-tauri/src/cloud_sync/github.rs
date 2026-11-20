//! GitHub REST API 的最小封装：统一 UA、令牌与错误处理。
//! 注意：所有请求必须带 User-Agent，否则 GitHub 返回 403。

use serde_json::Value;

pub const API: &str = "https://api.github.com";
pub const UPLOADS: &str = "https://uploads.github.com";
pub const UA: &str = concat!("QookiX-Launcher/", env!("CARGO_PKG_VERSION"));

/// 带令牌的 GET（返回 JSON）；404 返回 Ok(None) 供「存在性检查」使用。
pub async fn get(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<Option<Value>, String> {
    let resp = client
        .get(url)
        .header("User-Agent", UA)
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("GitHub 请求失败: {e}"))?;
    match resp.status().as_u16() {
        200 => Ok(Some(
            resp.json().await.map_err(|e| format!("解析响应失败: {e}"))?,
        )),
        404 => Ok(None),
        s => Err(format!("GitHub 返回 HTTP {s}: {}", short_body(resp).await)),
    }
}

/// Read a complete GitHub collection, including the pages used for quota cleanup.
pub async fn get_collection(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<Option<Vec<Value>>, String> {
    collect_pages(|page| async move {
        let next = format!("{url}?per_page=100&page={page}");
        get(client, &next, token).await
    }).await
}

async fn collect_pages<F, Request>(request: F) -> Result<Option<Vec<Value>>, String>
where
    F: Fn(u64) -> Request,
    Request: std::future::Future<Output = Result<Option<Value>, String>>,
{
    let mut output = Vec::new();
    for page in 1.. {
        let Some(value) = request(page).await? else {
            return if page == 1 { Ok(None) } else { Err("GitHub 列表在分页读取期间消失".into()) };
        };
        let entries = match value {
            Value::Array(entries) => entries,
            _ => return Err("GitHub 列表响应格式无效".into()),
        };
        let done = entries.len() < 100;
        output.extend(entries);
        if done {
            return Ok(Some(output));
        }
    }
    unreachable!()
}

/// 带令牌的 POST / PATCH / DELETE（JSON body），返回响应 JSON（204 返回 null）。
pub async fn send_json(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<Value, String> {
    let mut req = client
        .request(method, url)
        .header("User-Agent", UA)
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| format!("GitHub 请求失败: {e}"))?;
    let status = resp.status().as_u16();
    if status == 204 {
        return Ok(Value::Null);
    }
    if !(200..300).contains(&status) {
        return Err(format!("GitHub 返回 HTTP {status}: {}", short_body(resp).await));
    }
    resp.json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))
}

async fn short_body(resp: reqwest::Response) -> String {
    resp.text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn collects_following_pages_and_detects_an_incomplete_listing() {
        let entries = collect_pages(|page| async move {
            Ok(Some(Value::Array(match page {
                1 => (0..100).map(Value::from).collect(),
                2 => vec![Value::from(100), Value::from(101)],
                page => panic!("unexpected page {page}"),
            })))
        }).await.unwrap().unwrap();
        assert_eq!(entries.len(), 102);
        assert_eq!(entries.last(), Some(&Value::from(101)));

        let interrupted = collect_pages(|page| async move {
            Ok((page == 1).then(|| Value::Array(vec![Value::Null; 100])))
        }).await;
        assert!(interrupted.is_err());
    }
}
