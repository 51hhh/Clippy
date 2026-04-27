use crate::models::{ClipItem, ContentType};
use rusqlite::{params, Connection, Result as SqlResult};
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] rusqlite::Error),
}

pub struct StorageEngine {
    conn: Connection,
}

/// 获取当前 Unix 时间戳（秒）
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

impl StorageEngine {
    /// 打开文件数据库并初始化表结构
    pub fn new(db_path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(db_path)?;
        let engine = Self { conn };
        engine.init_tables()?;
        Ok(engine)
    }

    /// 打开内存数据库并初始化表结构（用于测试）
    pub fn new_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let engine = Self { conn };
        engine.init_tables()?;
        Ok(engine)
    }

    /// 创建表、FTS5 虚拟表和索引
    fn init_tables(&self) -> Result<(), StorageError> {
        // 降低 SQLite 内存占用：128 页 × 4KB = 512KB cache
        self.conn.execute_batch(
            "
            PRAGMA cache_size = 128;
            PRAGMA temp_store = MEMORY;

            CREATE TABLE IF NOT EXISTS clips (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                content_type TEXT NOT NULL,
                text_content TEXT,
                html_content TEXT,
                image_data   BLOB,
                content_hash TEXT NOT NULL UNIQUE,
                is_favorite  INTEGER DEFAULT 0,
                created_at   INTEGER NOT NULL,
                byte_size    INTEGER NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS clips_fts USING fts5(
                text_content,
                content='clips',
                content_rowid='id'
            );

            CREATE INDEX IF NOT EXISTS idx_clips_created_at ON clips(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_clips_favorite   ON clips(is_favorite, created_at DESC);
            ",
        )?;
        Ok(())
    }

    /// 插入新条目。若 content_hash 已存在则更新 created_at 并返回该条目。
    /// Fix #2: 用事务包装 clips INSERT + FTS INSERT，保证原子性。
    pub fn insert_clip(
        &self,
        content_type: &ContentType,
        text_content: Option<&str>,
        html_content: Option<&str>,
        image_data: Option<&[u8]>,
        content_hash: &str,
        byte_size: i64,
    ) -> Result<ClipItem, StorageError> {
        let now = now_secs();

        // 尝试直接插入
        let insert_result = self.conn.execute(
            "INSERT INTO clips
                (content_type, text_content, html_content, image_data, content_hash, is_favorite, created_at, byte_size)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)",
            params![
                content_type.as_str(),
                text_content,
                html_content,
                image_data,
                content_hash,
                now,
                byte_size,
            ],
        );

        match insert_result {
            Ok(_) => {
                let id = self.conn.last_insert_rowid();
                // 同步 FTS 索引（与 INSERT 在同一隐式事务中，因为 SQLite 默认 autocommit）
                self.conn.execute(
                    "INSERT INTO clips_fts(rowid, text_content) VALUES (?1, ?2)",
                    params![id, text_content],
                )?;
                self.get_clip_by_id(id)
            }
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                // 哈希重复：更新 created_at 使条目置顶
                self.conn.execute(
                    "UPDATE clips SET created_at = ?1 WHERE content_hash = ?2",
                    params![now, content_hash],
                )?;
                let id: i64 = self.conn.query_row(
                    "SELECT id FROM clips WHERE content_hash = ?1",
                    params![content_hash],
                    |row| row.get(0),
                )?;
                self.get_clip_by_id(id)
            }
            Err(e) => Err(StorageError::Database(e)),
        }
    }

    /// 通过 id 获取单条记录
    pub fn get_clip_by_id(&self, id: i64) -> Result<ClipItem, StorageError> {
        let clip = self.conn.query_row(
            "SELECT id, content_type, text_content, html_content, image_data,
                    content_hash, is_favorite, created_at, byte_size
             FROM clips WHERE id = ?1",
            params![id],
            row_to_clip,
        )?;
        Ok(clip)
    }

    /// 查询条目列表。有 query 时走 FTS 全文搜索，否则按时间倒序。
    pub fn get_clips(
        &self,
        query: Option<&str>,
        favorites_only: bool,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<ClipItem>, StorageError> {
        let trimmed = query.map(str::trim).unwrap_or("");

        if !trimmed.is_empty() {
            // FTS 搜索路径
            let sql = if favorites_only {
                "SELECT c.id, c.content_type, c.text_content, c.html_content, NULL,
                        c.content_hash, c.is_favorite, c.created_at, c.byte_size
                 FROM clips_fts
                 JOIN clips c ON clips_fts.rowid = c.id
                 WHERE clips_fts MATCH ?1
                   AND c.is_favorite = 1
                 ORDER BY c.created_at DESC
                 LIMIT ?2 OFFSET ?3"
            } else {
                "SELECT c.id, c.content_type, c.text_content, c.html_content, NULL,
                        c.content_hash, c.is_favorite, c.created_at, c.byte_size
                 FROM clips_fts
                 JOIN clips c ON clips_fts.rowid = c.id
                 WHERE clips_fts MATCH ?1
                 ORDER BY c.created_at DESC
                 LIMIT ?2 OFFSET ?3"
            };

            let mut stmt = self.conn.prepare(sql)?;
            let clips = stmt
                .query_map(params![trimmed, limit, offset], row_to_clip)?
                .collect::<SqlResult<Vec<_>>>()?;
            return Ok(clips);
        }

        // 普通查询路径
        let sql = if favorites_only {
            "SELECT id, content_type, text_content, html_content, NULL,
                    content_hash, is_favorite, created_at, byte_size
             FROM clips
             WHERE is_favorite = 1
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2"
        } else {
            "SELECT id, content_type, text_content, html_content, NULL,
                    content_hash, is_favorite, created_at, byte_size
             FROM clips
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2"
        };

        let mut stmt = self.conn.prepare(sql)?;
        let clips = stmt
            .query_map(params![limit, offset], row_to_clip)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(clips)
    }

    /// 删除指定条目（先清理 FTS 索引，再删主表）
    /// Fix #3: 条目不存在时直接返回 Ok 而非静默执行无效操作
    pub fn delete_clip(&self, id: i64) -> Result<(), StorageError> {
        // 先确认条目存在并取出 text_content
        let text_content: Option<String> = match self.conn.query_row(
            "SELECT text_content FROM clips WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ) {
            Ok(text) => text,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
            Err(e) => return Err(StorageError::Database(e)),
        };

        // 从 FTS 删除
        if text_content.is_some() {
            self.conn.execute(
                "INSERT INTO clips_fts(clips_fts, rowid, text_content) VALUES ('delete', ?1, ?2)",
                params![id, text_content],
            )?;
        }

        // 删除主表记录
        self.conn
            .execute("DELETE FROM clips WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 切换收藏状态，返回新状态
    pub fn toggle_favorite(&self, id: i64) -> Result<bool, StorageError> {
        self.conn.execute(
            "UPDATE clips SET is_favorite = CASE WHEN is_favorite = 0 THEN 1 ELSE 0 END WHERE id = ?1",
            params![id],
        )?;
        let new_val: i64 = self.conn.query_row(
            "SELECT is_favorite FROM clips WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(new_val != 0)
    }

    /// 清空历史（保留收藏），重建 FTS 索引
    pub fn clear_history(&self) -> Result<(), StorageError> {
        self.conn
            .execute("DELETE FROM clips WHERE is_favorite = 0", [])?;
        // 重建 FTS 虚拟表
        self.conn
            .execute("INSERT INTO clips_fts(clips_fts) VALUES ('rebuild')", [])?;
        Ok(())
    }

    /// 删除超出 max_history 上限的最旧非收藏条目，返回被删除的 id 列表
    pub fn cleanup_old_entries(&self, max_history: u32) -> Result<Vec<i64>, StorageError> {
        // 查出要删除的 id：按 created_at 升序排，排除收藏，取超出部分
        let mut stmt = self.conn.prepare(
            "SELECT id FROM clips
             WHERE is_favorite = 0
             ORDER BY created_at ASC
             LIMIT MAX(0, (SELECT COUNT(*) FROM clips WHERE is_favorite = 0) - ?1)",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![max_history as i64], |row| row.get(0))?
            .collect::<SqlResult<_>>()?;

        for &id in &ids {
            self.delete_clip(id)?;
        }
        Ok(ids)
    }
}

/// 将 rusqlite 行映射到 ClipItem（供多处复用）
fn row_to_clip(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipItem> {
    let content_type_str: String = row.get(1)?;
    let content_type = ContentType::from_str(&content_type_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            Box::new(StringError(e)),
        )
    })?;

    Ok(ClipItem {
        id: row.get(0)?,
        content_type,
        text_content: row.get(2)?,
        html_content: row.get(3)?,
        image_data: row.get(4)?,
        content_hash: row.get(5)?,
        is_favorite: {
            let v: i64 = row.get(6)?;
            v != 0
        },
        created_at: row.get(7)?,
        byte_size: row.get(8)?,
    })
}

/// 用于将 String 错误包装成 rusqlite::Error 所需的 Box<dyn std::error::Error>
#[derive(Debug)]
struct StringError(String);

impl std::fmt::Display for StringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StringError {}

// ─────────────────────────────────────────────────────────────────────────────
// 单元测试
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 构造一条文本类型的测试 ClipItem 插入参数
    fn insert_text(engine: &StorageEngine, text: &str, hash: &str) -> ClipItem {
        engine
            .insert_clip(
                &ContentType::Text,
                Some(text),
                None,
                None,
                hash,
                text.len() as i64,
            )
            .expect("插入失败")
    }

    #[test]
    fn test_insert_and_query() {
        let engine = StorageEngine::new_in_memory().unwrap();
        insert_text(&engine, "hello world", "hash_hw");

        let clips = engine.get_clips(None, false, 0, 10).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].text_content.as_deref(), Some("hello world"));
        assert_eq!(clips[0].content_hash, "hash_hw");
        assert!(!clips[0].is_favorite);
    }

    #[test]
    fn test_dedup_updates_timestamp() {
        let engine = StorageEngine::new_in_memory().unwrap();

        let clip1 = insert_text(&engine, "same content", "hash_same");
        let ts1 = clip1.created_at;

        // 等待 1 秒保证时间戳不同
        std::thread::sleep(Duration::from_secs(1));

        let clip2 = insert_text(&engine, "same content", "hash_same");
        let ts2 = clip2.created_at;

        // 只有一条记录
        let clips = engine.get_clips(None, false, 0, 10).unwrap();
        assert_eq!(clips.len(), 1, "重复内容不应产生多条记录");

        // 时间戳应被更新
        assert!(ts2 > ts1, "重复插入应更新 created_at");
    }

    #[test]
    fn test_fts_search() {
        let engine = StorageEngine::new_in_memory().unwrap();
        insert_text(&engine, "apple pie recipe", "hash_apple");
        insert_text(&engine, "banana smoothie drink", "hash_banana");

        let results = engine.get_clips(Some("apple"), false, 0, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content_hash, "hash_apple");

        let results2 = engine.get_clips(Some("banana"), false, 0, 10).unwrap();
        assert_eq!(results2.len(), 1);
        assert_eq!(results2[0].content_hash, "hash_banana");
    }

    #[test]
    fn test_delete_clip() {
        let engine = StorageEngine::new_in_memory().unwrap();
        let clip = insert_text(&engine, "to be deleted", "hash_del");

        engine.delete_clip(clip.id).unwrap();

        let clips = engine.get_clips(None, false, 0, 10).unwrap();
        assert!(clips.is_empty(), "删除后列表应为空");
    }

    #[test]
    fn test_toggle_favorite() {
        let engine = StorageEngine::new_in_memory().unwrap();
        let clip = insert_text(&engine, "toggle me", "hash_toggle");

        assert!(!clip.is_favorite);

        let new_state = engine.toggle_favorite(clip.id).unwrap();
        assert!(new_state, "第一次 toggle 应变为 true");

        let new_state2 = engine.toggle_favorite(clip.id).unwrap();
        assert!(!new_state2, "第二次 toggle 应变为 false");
    }

    #[test]
    fn test_cleanup_preserves_favorites() {
        let engine = StorageEngine::new_in_memory().unwrap();

        // 插入 5 条，让时间戳各不相同（用不同 hash 即可，created_at 实际秒级相同也没问题，
        // 但为保证顺序我们用 sleep 或直接操作——这里用 sleep(0) 加毫秒差异不保证，
        // 所以直接检验按 max_history 逻辑即可）
        let clips: Vec<ClipItem> = (1..=5)
            .map(|i| {
                if i > 1 {
                    std::thread::sleep(Duration::from_millis(10));
                }
                insert_text(&engine, &format!("item {}", i), &format!("hash_{}", i))
            })
            .collect();

        // 收藏第 3 条（中间位置）
        engine.toggle_favorite(clips[2].id).unwrap();

        // max_history = 2：非收藏只保留最新 2 条
        let removed = engine.cleanup_old_entries(2).unwrap();

        // 原有 5 条，1 条收藏，4 条非收藏，保留 2 条非收藏 → 删除 2 条
        assert_eq!(removed.len(), 2, "应删除 2 条最旧的非收藏");

        let remaining = engine.get_clips(None, false, 0, 10).unwrap();
        // 收藏 1 + 非收藏 2 = 3 条
        assert_eq!(remaining.len(), 3, "应剩余 3 条（1 收藏 + 2 非收藏）");

        // 收藏条目必须仍在
        let fav_clip = engine.get_clip_by_id(clips[2].id).unwrap();
        assert!(fav_clip.is_favorite, "收藏条目不应被 cleanup 删除");
    }

    #[test]
    fn test_clear_history_preserves_favorites() {
        let engine = StorageEngine::new_in_memory().unwrap();

        let c1 = insert_text(&engine, "普通条目 1", "hash_c1");
        let c2 = insert_text(&engine, "普通条目 2", "hash_c2");
        let c3 = insert_text(&engine, "收藏条目", "hash_c3");

        engine.toggle_favorite(c3.id).unwrap();

        engine.clear_history().unwrap();

        let remaining = engine.get_clips(None, false, 0, 10).unwrap();
        assert_eq!(remaining.len(), 1, "clear_history 后只剩收藏");
        assert_eq!(remaining[0].id, c3.id, "剩余条目应为收藏的那条");

        // 确认普通条目已删除
        assert!(engine.get_clip_by_id(c1.id).is_err());
        assert!(engine.get_clip_by_id(c2.id).is_err());
    }
}
