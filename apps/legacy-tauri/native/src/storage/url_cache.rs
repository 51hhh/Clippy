use super::{now_secs, StorageEngine, StorageError};
use crate::models::UrlMeta;
use rusqlite::params;

pub(super) const URL_META_TTL_SECS: i64 = 7 * 86400;
pub(super) const MAX_URL_META_ENTRIES: i64 = 512;

impl StorageEngine {
    /// 查询 URL 元数据缓存（7 天内有效）。
    pub fn get_url_meta(&self, url: &str) -> Result<Option<UrlMeta>, StorageError> {
        let max_age = now_secs() - URL_META_TTL_SECS;
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

    /// 写入 URL 元数据缓存，并顺手清理过期与超限行。
    ///
    /// 仅在读取时忽略过期记录会让数据库随用户复制过的不同链接永久增长；链接预览只是缓存，
    /// 保留最近 512 条已经远超主历史默认容量，且能把磁盘占用稳定限制在可预期范围内。
    pub fn set_url_meta(&self, meta: &UrlMeta) -> Result<(), StorageError> {
        let now = now_secs();
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "INSERT OR REPLACE INTO url_meta_cache (url, title, description, favicon, site_name, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                meta.url,
                meta.title,
                meta.description,
                meta.favicon,
                meta.site_name,
                now,
            ],
        )?;
        self.conn.execute(
            "DELETE FROM url_meta_cache WHERE fetched_at <= ?1",
            params![now - URL_META_TTL_SECS],
        )?;
        self.conn.execute(
            "DELETE FROM url_meta_cache
             WHERE rowid IN (
                 SELECT rowid FROM url_meta_cache
                 ORDER BY fetched_at DESC, rowid DESC
                 LIMIT -1 OFFSET ?1
             )",
            params![MAX_URL_META_ENTRIES],
        )?;
        tx.commit()?;
        Ok(())
    }
}
