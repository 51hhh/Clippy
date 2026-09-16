use super::{now_secs, StorageEngine, StorageError};
use rusqlite::{params, Result as SqlResult};

impl StorageEngine {
    /// 删除超出 max_history 上限的最旧非收藏条目，返回被删除的 id 列表。
    pub fn cleanup_old_entries(&self, max_history: u32) -> Result<Vec<i64>, StorageError> {
        // 设置页约定 0 为不限；不能把它当成需要保留零条。
        if max_history == 0 {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT id FROM clips
             WHERE is_favorite = 0
             ORDER BY use_order ASC, id ASC
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

    /// 批量删除：FTS 清理 + 主表 DELETE + 孤儿译文清理必须原子，
    /// 否则中途失败会留下搜得到但已不存在的条目，或删不掉的译文。
    fn delete_entries(&self, ids: Vec<i64>) -> Result<Vec<i64>, StorageError> {
        if ids.is_empty() {
            return Ok(ids);
        }

        let tx = self.conn.unchecked_transaction()?;

        let mut delete_fts = self.conn.prepare_cached(
            "INSERT INTO clips_fts(clips_fts, rowid, text_content) VALUES ('delete', ?1, ?2)",
        )?;
        // 无限历史切回有限上限可能一次删除数万条。每条 SQL 控制在 500 个参数，
        // 同时兼容旧 SQLite 的 999 上限；所有分块仍共享同一个事务，不能部分提交。
        for chunk in ids.chunks(500) {
            let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let select_fts = format!(
                "SELECT id, text_content FROM clips WHERE id IN ({placeholders}) AND text_content IS NOT NULL"
            );
            let mut select = self.conn.prepare(&select_fts)?;
            let fts_entries = select
                .query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<SqlResult<Vec<_>>>()?;
            for (id, text) in fts_entries {
                delete_fts.execute(params![id, text])?;
            }
            let delete_clips = format!("DELETE FROM clips WHERE id IN ({placeholders})");
            self.conn
                .execute(&delete_clips, rusqlite::params_from_iter(chunk.iter()))?;
        }
        // 被清理条目的译文不该留在库里。
        self.purge_orphan_translations()?;
        tx.commit()?;
        Ok(ids)
    }
}
