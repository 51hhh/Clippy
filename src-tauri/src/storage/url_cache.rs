use super::{now_secs, StorageEngine, StorageError};
use crate::models::UrlMeta;
use rusqlite::params;

impl StorageEngine {
    /// 查询 URL 元数据缓存（7 天内有效）。
    pub fn get_url_meta(&self, url: &str) -> Result<Option<UrlMeta>, StorageError> {
        let max_age = now_secs() - 7 * 86400;
        let result = self.conn.query_row(
            "SELECT url, title, description, favicon, site_name FROM url_meta_cache
             WHERE url = ?1 AND fetched_at > ?2",
            params![url, max_age],
            |row| {
                Ok(UrlMeta {
                    url: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    favicon: row.get(3)?,
                    site_name: row.get(4)?,
                })
            },
        );
        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(StorageError::Database(error)),
        }
    }

    /// 写入 URL 元数据缓存。
    pub fn set_url_meta(&self, meta: &UrlMeta) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO url_meta_cache (url, title, description, favicon, site_name, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                meta.url,
                meta.title,
                meta.description,
                meta.favicon,
                meta.site_name,
                now_secs(),
            ],
        )?;
        Ok(())
    }
}
