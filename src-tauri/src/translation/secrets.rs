//! 翻译凭据存储。只写系统 Secret Service/keychain，任何情况下都不回退到配置文件。
//!
//! 有道官方 API 需要 appKey + appSecret 两个字段，因此每个服务最多占两条 keyring 记录：
//! 用户名 `{provider}` 存主密钥，`{provider}.secret` 存第二字段。主密钥沿用旧用户名，
//! 老版本写入的密钥升级后仍然能被读到。

use super::types::{ProviderCredentials, TranslationError, TranslationProvider};

const KEYRING_SERVICE: &str = "com.clippy.app.translation";
/// 第二字段的用户名后缀。provider 名里不含点，所以不会和主密钥撞名。
const SECRET_SUFFIX: &str = ".secret";

/// 凭据的两个字段。分开成独立记录而不是拼一个 JSON，
/// 这样用户在系统密钥管理器里能看清自己存了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Key,
    Secret,
}

fn username(provider: TranslationProvider, field: Field) -> String {
    match field {
        Field::Key => provider.as_str().to_string(),
        Field::Secret => format!("{}{SECRET_SUFFIX}", provider.as_str()),
    }
}

fn entry(provider: TranslationProvider, field: Field) -> Result<keyring::Entry, TranslationError> {
    keyring::Entry::new(KEYRING_SERVICE, &username(provider, field))
        .map_err(|_| TranslationError::KeyringUnavailable)
}

fn read(provider: TranslationProvider, field: Field) -> Result<Option<String>, TranslationError> {
    match entry(provider, field)?.get_password() {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(TranslationError::KeyringUnavailable),
    }
}

fn write(provider: TranslationProvider, field: Field, value: &str) -> Result<(), TranslationError> {
    entry(provider, field)?
        .set_password(value)
        .map_err(|_| TranslationError::KeyringUnavailable)
}

fn remove(provider: TranslationProvider, field: Field) -> Result<(), TranslationError> {
    match entry(provider, field)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(TranslationError::KeyringUnavailable),
    }
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// 写入一个服务的凭据。第二字段为空时删除旧记录，避免残留上一次的 appSecret。
pub fn set_credentials(
    provider: TranslationProvider,
    api_key: &str,
    api_secret: Option<&str>,
) -> Result<(), TranslationError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(TranslationError::MissingApiKey);
    }
    let api_secret = clean(api_secret);
    // 有道这类双字段服务只填一半时直接拒绝：存下去只会在翻译时才报错，
    // 而那时用户已经离开设置页，很难把错误和自己刚才的输入联系起来。
    if provider.requires_api_secret() && api_secret.is_none() {
        return Err(TranslationError::IncompleteCredentials);
    }

    write(provider, Field::Key, api_key)?;
    match api_secret {
        Some(secret) => write(provider, Field::Secret, secret),
        None => remove(provider, Field::Secret),
    }
}

pub fn get_credentials(
    provider: TranslationProvider,
) -> Result<ProviderCredentials, TranslationError> {
    Ok(ProviderCredentials::new(
        read(provider, Field::Key)?,
        read(provider, Field::Secret)?,
    ))
}

/// 设置页据此显示「已配置」。只填一半的服务不算已配置。
pub fn has_credentials(provider: TranslationProvider) -> Result<bool, TranslationError> {
    Ok(get_credentials(provider)?.complete_for(provider))
}

pub fn delete_credentials(provider: TranslationProvider) -> Result<(), TranslationError> {
    remove(provider, Field::Key)?;
    remove(provider, Field::Secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_keys_are_rejected_before_touching_keyring() {
        let error = set_credentials(TranslationProvider::LibreTranslate, "  ", None).unwrap_err();
        assert_eq!(error, TranslationError::MissingApiKey);
    }

    #[test]
    fn dual_field_providers_reject_a_half_filled_pair() {
        assert_eq!(
            set_credentials(TranslationProvider::Youdao, "app-key", None),
            Err(TranslationError::IncompleteCredentials)
        );
        assert_eq!(
            set_credentials(TranslationProvider::Youdao, "app-key", Some("   ")),
            Err(TranslationError::IncompleteCredentials)
        );
    }

    #[test]
    fn providers_use_distinct_stable_usernames() {
        // 主密钥用户名保持为 provider 名，老版本写入的密钥升级后仍能读到。
        for provider in TranslationProvider::all() {
            assert_eq!(username(provider, Field::Key), provider.as_str());
            assert_eq!(
                username(provider, Field::Secret),
                format!("{}.secret", provider.as_str())
            );
        }

        let mut usernames: Vec<String> = TranslationProvider::all()
            .into_iter()
            .flat_map(|provider| {
                [
                    username(provider, Field::Key),
                    username(provider, Field::Secret),
                ]
            })
            .collect();
        let total = usernames.len();
        usernames.sort();
        usernames.dedup();
        assert_eq!(usernames.len(), total, "keyring 用户名必须互不重复");
    }
}
