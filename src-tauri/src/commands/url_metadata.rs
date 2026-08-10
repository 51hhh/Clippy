use super::AppState;
use crate::models::UrlMeta;
use tauri::State;

/// 抓取 URL 的 Open Graph 元数据并写入缓存。
#[tauri::command]
pub async fn fetch_url_meta(url: String, state: State<'_, AppState>) -> Result<UrlMeta, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("仅支持 http/https URL".to_string());
    }
    if is_private_url(&url) {
        return Err("不允许请求内网地址".to_string());
    }

    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        if let Ok(Some(cached)) = storage.get_url_meta(&url) {
            return Ok(cached);
        }
    }

    let url_clone = url.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || -> Result<UrlMeta, String> {
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .build(),
        );
        let response = agent
            .get(&url_clone)
            .header("User-Agent", "Clippy/0.1 (Link Preview)")
            .call()
            .map_err(|e| format!("请求失败: {}", e))?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !content_type.contains("text/html") {
            return Err("非 HTML 页面".to_string());
        }

        let body = response
            .into_body()
            .with_config()
            .limit(1_048_576)
            .read_to_string()
            .map_err(|e| format!("读取失败: {}", e))?;
        Ok(parse_og_meta(&url_clone, &body))
    })
    .await
    .map_err(|e| format!("线程异常: {}", e))??;

    {
        let storage = state.storage.lock().map_err(|e| e.to_string())?;
        let _ = storage.set_url_meta(&meta);
    }
    Ok(meta)
}

fn parse_og_meta(url: &str, html: &str) -> UrlMeta {
    let get_meta = |property: &str| -> Option<String> {
        let pattern = format!(
            r#"<meta[^>]+(?:property|name)=["']{}["'][^>]+content=["']([^"']*)["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&pattern) {
            if let Some(captures) = re.captures(html) {
                let value = captures.get(1)?.as_str().trim().to_string();
                if !value.is_empty() {
                    return Some(html_decode(&value));
                }
            }
        }

        let reverse_pattern = format!(
            r#"<meta[^>]+content=["']([^"']*)["'][^>]+(?:property|name)=["']{}["']"#,
            regex_lite::escape(property)
        );
        if let Ok(re) = regex_lite::Regex::new(&reverse_pattern) {
            if let Some(captures) = re.captures(html) {
                let value = captures.get(1)?.as_str().trim().to_string();
                if !value.is_empty() {
                    return Some(html_decode(&value));
                }
            }
        }
        None
    };

    let title = get_meta("og:title").or_else(|| {
        let re = regex_lite::Regex::new(r"<title[^>]*>([^<]+)</title>").ok()?;
        let captures = re.captures(html)?;
        let value = captures.get(1)?.as_str().trim().to_string();
        if value.is_empty() {
            None
        } else {
            Some(html_decode(&value))
        }
    });
    let description = get_meta("og:description").or_else(|| get_meta("description"));
    let site_name = get_meta("og:site_name");

    let favicon = {
        let re = regex_lite::Regex::new(
            r#"<link[^>]+rel=["'](?:icon|shortcut icon)["'][^>]+href=["']([^"']+)["']"#,
        )
        .ok();
        re.and_then(|regex| regex.captures(html))
            .and_then(|captures| captures.get(1))
            .map(|favicon_match| {
                let href = favicon_match.as_str().trim();
                if href.starts_with("http") {
                    href.to_string()
                } else if href.starts_with("//") {
                    format!("https:{}", href)
                } else {
                    let base = url.split('/').take(3).collect::<Vec<_>>().join("/");
                    if href.starts_with('/') {
                        format!("{}{}", base, href)
                    } else {
                        format!("{}/{}", base, href)
                    }
                }
            })
            .or_else(|| {
                let base = url.split('/').take(3).collect::<Vec<_>>().join("/");
                Some(format!("{}/favicon.ico", base))
            })
    };

    UrlMeta {
        url: url.to_string(),
        title,
        description,
        favicon,
        site_name,
    }
}

fn html_decode(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

fn is_private_url(url: &str) -> bool {
    let after_scheme = url.split("://").nth(1).unwrap_or("");
    let host_port = after_scheme.split('/').next().unwrap_or("");
    let host = if host_port.starts_with('[') {
        host_port
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.to_lowercase();
    host == "localhost"
        || host == "::1"
        || host == "0.0.0.0"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("172.16.")
        || host.starts_with("172.17.")
        || host.starts_with("172.18.")
        || host.starts_with("172.19.")
        || host.starts_with("172.20.")
        || host.starts_with("172.21.")
        || host.starts_with("172.22.")
        || host.starts_with("172.23.")
        || host.starts_with("172.24.")
        || host.starts_with("172.25.")
        || host.starts_with("172.26.")
        || host.starts_with("172.27.")
        || host.starts_with("172.28.")
        || host.starts_with("172.29.")
        || host.starts_with("172.30.")
        || host.starts_with("172.31.")
        || host.starts_with("169.254.")
        || host.starts_with("fd")
        || host.starts_with("fe80")
}
