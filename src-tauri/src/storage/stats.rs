use super::{StorageEngine, StorageError};

impl StorageEngine {
    /// 获取剪贴板统计信息。
    pub fn get_stats(&self) -> Result<serde_json::Value, StorageError> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM clips", [], |row| row.get(0))?;
        let favorites: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE is_favorite = 1",
            [],
            |row| row.get(0),
        )?;
        let text_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE content_type = 'text'",
            [],
            |row| row.get(0),
        )?;
        let html_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE content_type = 'html'",
            [],
            |row| row.get(0),
        )?;
        let image_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE content_type = 'image'",
            [],
            |row| row.get(0),
        )?;
        let sensitive_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM clips WHERE is_sensitive = 1",
            [],
            |row| row.get(0),
        )?;
        let total_bytes: i64 =
            self.conn
                .query_row("SELECT COALESCE(SUM(byte_size), 0) FROM clips", [], |row| {
                    row.get(0)
                })?;
        let db_size: i64 = self.conn.query_row(
            "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
            [],
            |row| row.get(0),
        )?;

        Ok(serde_json::json!({
            "total": total,
            "favorites": favorites,
            "text_count": text_count,
            "html_count": html_count,
            "image_count": image_count,
            "sensitive_count": sensitive_count,
            "total_bytes": total_bytes,
            "db_size": db_size,
        }))
    }
}
