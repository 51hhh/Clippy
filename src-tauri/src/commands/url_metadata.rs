use super::AppState;
use crate::models::UrlMeta;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use tauri::State;
use url::{Host, Url};

/// 抓取 URL 的 Open Graph 元数据并写入缓存。
#[tauri::command]
pub async fn fetch_url_meta(url: String, state: State<'_, AppState>) -> Result<UrlMeta, String> {
    validate_url_syntax(&url)?;

    let url_clone = url.clone();
    let storage = state.storage.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || -> Result<UrlMeta, String> {
        if is_private_url(&url_clone) {
            return Err("不允许请求内网地址".to_string());
        }
        {
            let storage = storage.lock().map_err(|error| error.to_string())?;
            if let Ok(Some(cached)) = storage.get_url_meta(&url_clone) {
                return Ok(cached);
            }
        }
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .max_redirects(0)
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

    let storage = state.storage.lock().map_err(|error| error.to_string())?;
    let _ = storage.set_url_meta(&meta);
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
    let Ok(parsed) = Url::parse(url) else {
        return true;
    };
    let Some(host) = parsed.host() else {
        return true;
    };
    match host {
        Host::Ipv4(address) => is_private_ip(IpAddr::V4(address)),
        Host::Ipv6(address) => is_private_ip(IpAddr::V6(address)),
        Host::Domain(name) => {
            let normalized = name.trim_end_matches('.').to_ascii_lowercase();
            if is_private_domain_name(&normalized) {
                return true;
            }
            let port = parsed.port_or_known_default().unwrap_or(443);
            (normalized.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| {
                    addresses
                        .into_iter()
                        .any(|address| is_private_ip(address.ip()))
                })
                .unwrap_or(false)
        }
    }
}

fn validate_url_syntax(url: &str) -> Result<(), String> {
    if url.is_empty() || url != url.trim() {
        return Err("URL 不能为空或包含首尾空白".to_string());
    }
    let parsed = Url::parse(url).map_err(|_| "仅支持有效的 http/https URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("仅支持不含凭据的 http/https URL".to_string());
    }
    let private_literal = match parsed.host() {
        Some(Host::Ipv4(address)) => is_private_ip(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => is_private_ip(IpAddr::V6(address)),
        Some(Host::Domain(name)) => {
            is_private_domain_name(&name.trim_end_matches('.').to_ascii_lowercase())
        }
        None => true,
    };
    if private_literal {
        return Err("不允许请求内网地址".to_string());
    }
    Ok(())
}

fn is_private_domain_name(name: &str) -> bool {
    name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
}

fn is_private_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => is_private_ipv4(ip),
        IpAddr::V6(ip) => is_private_ipv6(ip),
    }
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0)
        || (a == 192 && b == 168)
        || (a == 192 && b == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    if ip.is_loopback() || ip.is_unspecified() || segments[0] & 0xfe00 == 0xfc00 {
        return true;
    }
    if segments[0] & 0xffc0 == 0xfe80 || (segments[0] == 0x2001 && segments[1] == 0x0db8) {
        return true;
    }
    // IPv4-mapped IPv6 addresses must use the same private-range policy.
    if segments[..5].iter().all(|segment| *segment == 0) && segments[5] == 0xffff {
        let mapped = Ipv4Addr::new(
            (segments[6] >> 8) as u8,
            segments[6] as u8,
            (segments[7] >> 8) as u8,
            segments[7] as u8,
        );
        return is_private_ipv4(mapped);
    }
    segments[0] & 0xff00 == 0xff00
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_syntax_and_rejects_private_literal_ranges() {
        assert!(validate_url_syntax("https://example.invalid/path").is_ok());
        for url in [
            "http://127.0.0.1:8080",
            "https://[::1]/",
            "https://[fd00::1]/",
            "http://169.254.10.2/",
            "https://user:secret@example.com/",
            "file:///tmp/example",
            " https://example.com",
        ] {
            assert!(
                validate_url_syntax(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn classifies_mapped_and_reserved_addresses() {
        assert!(is_private_url("https://[::ffff:127.0.0.1]/"));
        assert!(is_private_url("https://224.0.0.1/"));
        assert!(is_private_url("https://198.51.100.10/"));
    }
}
