use super::types::{TranslationError, TranslationProvider};

const KEYRING_SERVICE: &str = "com.clippy.app.translation";

fn entry(provider: TranslationProvider) -> Result<keyring::Entry, TranslationError> {
    keyring::Entry::new(KEYRING_SERVICE, provider.as_str())
        .map_err(|_| TranslationError::KeyringUnavailable)
}

/// 将 API key 写入系统 Secret Service/keychain。绝不回退到配置文件。
pub fn set_api_key(provider: TranslationProvider, api_key: &str) -> Result<(), TranslationError> {
    if api_key.trim().is_empty() {
        return Err(TranslationError::MissingApiKey);
    }
    entry(provider)?
        .set_password(api_key)
        .map_err(|_| TranslationError::KeyringUnavailable)
}

pub fn get_api_key(provider: TranslationProvider) -> Result<Option<String>, TranslationError> {
    match entry(provider)?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(TranslationError::KeyringUnavailable),
    }
}

pub fn has_api_key(provider: TranslationProvider) -> Result<bool, TranslationError> {
    Ok(get_api_key(provider)?.is_some())
}

pub fn delete_api_key(provider: TranslationProvider) -> Result<(), TranslationError> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(TranslationError::KeyringUnavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_keys_are_rejected_before_touching_keyring() {
        let error = set_api_key(TranslationProvider::LibreTranslate, "  ").unwrap_err();
        assert_eq!(error, TranslationError::MissingApiKey);
    }

    #[test]
    fn providers_use_distinct_stable_usernames() {
        assert_ne!(
            TranslationProvider::LibreTranslate.as_str(),
            TranslationProvider::OpenAiCompatible.as_str()
        );
    }
}
