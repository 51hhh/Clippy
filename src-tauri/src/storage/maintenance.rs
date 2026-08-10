use super::{now_secs, StorageEngine, StorageError};
use rusqlite::{params, Result as SqlResult};

impl StorageEngine {
    /// 删除超出 max_history 上限的最旧非收藏条目，返回被删除的 id 列表。
    pub fn cleanup_old_entries(&self, max_history: u32) -> Result<Vec<i64>, StorageError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM clips
             WHERE is_favorite = 0
             ORDER BY created_at ASC
             LIMIT MAX(0, (SELECT COUNT(*) FROM clips WHERE is_favorite = 0) - ?1)",
        )?;
        let ids = stmt
            .query_map(params![max_history as i64], |row| row.get(0))?
            .collect::<SqlResult<Vec<i64>>>()?;
        self.delete_entries(ids)
    }

    /// 清理创建超过 ttl_secs 秒的非收藏敏感条目。
    pub fn purge_expired_sensitive(&self, ttl_secs: i64) -> Result<Vec<i64>, StorageError> {
        let cutoff = now_secs() - ttl_secs;
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM clips WHERE is_sensitive = 1 AND is_favorite = 0 AND created_at < ?1",
        )?;
        let ids = stmt
            .query_map(params![cutoff], |row| row.get(0))?
            .collect::<SqlResult<Vec<i64>>>()?;
        self.delete_entries(ids)
    }

    fn delete_entries(&self, ids: Vec<i64>) -> Result<Vec<i64>, StorageError> {
        if ids.is_empty() {
            return Ok(ids);
        }

        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let select_fts = format!(
            "SELECT id, text_content FROM clips WHERE id IN ({placeholders}) AND text_content IS NOT NULL"
        );
        let mut select = self.conn.prepare(&select_fts)?;
        let fts_entries = select
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<SqlResult<Vec<_>>>()?;

        let mut delete_fts = self.conn.prepare_cached(
            "INSERT INTO clips_fts(clips_fts, rowid, text_content) VALUES ('delete', ?1, ?2)",
        )?;
        for (id, text) in fts_entries {
            delete_fts.execute(params![id, text])?;
        }

        let delete_clips = format!("DELETE FROM clips WHERE id IN ({placeholders})");
        self.conn
            .execute(&delete_clips, rusqlite::params_from_iter(ids.iter()))?;
        Ok(ids)
    }
}
