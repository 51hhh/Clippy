//! 官方 API 与非官方 web 端点之间的选路。
//!
//! 仅供 DeepL/Google/Bing/有道 这类双路径服务使用；LibreTranslate 与 OpenAI-compatible
//! 自己处理密钥，不走这里。选路代价与决策背景见 docs/reference-project-guidance.md。

use super::super::types::{ProviderCredentials, TranslationError, TranslationProvider};

/// 本次请求实际要走的路径。
pub(super) enum Route<'a> {
    Official {
        key: &'a str,
        secret: Option<&'a str>,
    },
    Web,
}

pub(super) fn route<'a>(
    provider: TranslationProvider,
    credentials: &'a ProviderCredentials,
) -> Result<Route<'a>, TranslationError> {
    if credentials.complete_for(provider) {
        let key = credentials.key().ok_or(TranslationError::MissingApiKey)?;
        return Ok(Route::Official {
            key,
            secret: credentials.secret(),
        });
    }

    // 凭据只填了一半时用户以为自己在用官方 API。静默降级会把待译文本发到非官方端点，
    // 所以这里必须报错，让设置页把缺失的字段指出来。
    if !credentials.is_empty() {
        return Err(TranslationError::IncompleteCredentials);
    }

    if provider.requires_credentials() || provider.default_web_endpoint().is_empty() {
        return Err(TranslationError::MissingApiKey);
    }
    Ok(Route::Web)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_web(route: &Route<'_>) -> bool {
        matches!(route, Route::Web)
    }

    #[test]
    fn complete_credentials_choose_the_official_api() {
        let credentials = ProviderCredentials::new(Some("key".to_string()), None);
        match route(TranslationProvider::DeepL, &credentials).unwrap() {
            Route::Official { key, secret } => {
                assert_eq!(key, "key");
                assert_eq!(secret, None);
            }
            Route::Web => panic!("配置了密钥却走了 web 路径"),
        }

        let pair = ProviderCredentials::new(Some("app".to_string()), Some("secret".to_string()));
        match route(TranslationProvider::Youdao, &pair).unwrap() {
            Route::Official { key, secret } => {
                assert_eq!(key, "app");
                assert_eq!(secret, Some("secret"));
            }
            Route::Web => panic!("配置了完整凭据却走了 web 路径"),
        }
    }

    #[test]
    fn no_credentials_fall_back_to_the_web_endpoint() {
        let empty = ProviderCredentials::default();
        for provider in [
            TranslationProvider::DeepL,
            TranslationProvider::Google,
            TranslationProvider::Bing,
            TranslationProvider::Youdao,
        ] {
            assert!(is_web(&route(provider, &empty).unwrap()));
        }
    }

    #[test]
    fn half_filled_credentials_never_downgrade_silently() {
        let key_only = ProviderCredentials::new(Some("app".to_string()), None);
        assert_eq!(
            route(TranslationProvider::Youdao, &key_only)
                .err()
                .map(|error| error.code()),
            Some("incomplete_credentials")
        );

        let secret_only = ProviderCredentials::new(None, Some("secret".to_string()));
        assert_eq!(
            route(TranslationProvider::DeepL, &secret_only)
                .err()
                .map(|error| error.code()),
            Some("incomplete_credentials")
        );
    }

    #[test]
    fn providers_without_a_web_path_require_a_key() {
        let empty = ProviderCredentials::default();
        assert_eq!(
            route(TranslationProvider::OpenAiCompatible, &empty)
                .err()
                .map(|error| error.code()),
            Some("missing_api_key")
        );
    }
}
