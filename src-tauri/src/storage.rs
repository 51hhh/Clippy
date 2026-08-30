mod maintenance;
mod stats;
mod translation_history;
mod url_cache;

pub use translation_history::NewTranslation;

use crate::models::{ClipItem, ContentType};
use crate::private_files::{ensure_private_file, restrict_file};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("数据库操作失败: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("本地文件操作失败: {0}")]
    Io(#[from] std::io::Error),
}

impl StorageError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Database(_) => "database",
            Self::Io(_) => "io",
        }
    }
}

pub struct StorageEngine {
    conn: Connection,
}

/// 获取当前 Unix 时间戳（秒）
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn sanitize_search_query(query: &str) -> String {
    query
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
}

fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        match ch {
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(ch);
            }
            _ => pattern.push(ch),
        }
    }
    pattern.push('%');
    pattern
}

fn build_fts_prefix_query(query: &str) -> Option<String> {
    if query
        .chars()
        .any(|ch| !ch.is_alphanumeric() && !ch.is_whitespace())
    {
        return None;
    }

    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(|token| format!("{}*", token))
        .collect();

    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

impl StorageEngine {
    /// 打开文件数据库并初始化表结构
    pub fn new(db_path: &Path) -> Result<Self, StorageError> {
        ensure_private_file(db_path)?;
        let conn = Connection::open(db_path)?;
        let engine = Self { conn };
        engine.init_tables()?;
        engine.restrict_sidecar_permissions(db_path)?;
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
        // WAL 模式减少写阻塞，提升读写并发
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
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

            CREATE TABLE IF NOT EXISTS schema_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;

        // 迁移：添加 ocr_text 字段（已有数据库可能缺少此列）
        let has_ocr_col: bool = self
            .conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('clips') WHERE name='ocr_text'")?
            .query_row([], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false);
        if !has_ocr_col {
            self.conn
                .execute("ALTER TABLE clips ADD COLUMN ocr_text TEXT", [])?;
        }

        // 迁移：添加 is_sensitive 字段（敏感内容自动检测）
        let has_sensitive_col: bool = self
            .conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info('clips') WHERE name='is_sensitive'")?
            .query_row([], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false);
        if !has_sensitive_col {
            self.conn.execute(
                "ALTER TABLE clips ADD COLUMN is_sensitive INTEGER DEFAULT 0",
                [],
            )?;
        }

        // URL 元数据缓存表
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS url_meta_cache (
                url         TEXT PRIMARY KEY,
                title       TEXT,
                description TEXT,
                favicon     TEXT,
                site_name   TEXT,
                fetched_at  INTEGER NOT NULL
            );",
        )?;

        // 翻译历史表。clip_id = 0 表示不来自剪贴板条目（选区翻译或临时文本）：
        // SQLite 的 UNIQUE 不约束 NULL，用 0 作哨兵才能对这类记录同样去重。
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS translation_history (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                clip_id         INTEGER NOT NULL DEFAULT 0,
                provider        TEXT NOT NULL,
                source_language TEXT NOT NULL,
                target_language TEXT NOT NULL,
                source_hash     TEXT NOT NULL,
                source_text     TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                created_at      INTEGER NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_translation_history_unique
                ON translation_history(clip_id, provider, target_language, source_hash);
            CREATE INDEX IF NOT EXISTS idx_translation_history_created_at
                ON translation_history(created_at DESC);",
        )?;

        self.rebuild_fts_once("search_v2")?;

        Ok(())
    }

    fn restrict_sidecar_permissions(&self, db_path: &Path) -> Result<(), StorageError> {
        for suffix in ["-wal", "-shm"] {
            let mut sidecar_name = db_path.as_os_str().to_os_string();
            sidecar_name.push(suffix);
            let sidecar = PathBuf::from(sidecar_name);
            if sidecar.exists() {
                restrict_file(&sidecar)?;
            }
        }
        Ok(())
    }

    fn rebuild_fts_once(&self, version: &str) -> Result<(), StorageError> {
        let key = "fts_rebuild_version";
        let current = self
            .conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        if current.as_deref() != Some(version) {
            self.conn
                .execute("INSERT INTO clips_fts(clips_fts) VALUES ('rebuild')", [])?;
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value) VALUES (?1, ?2)",
                params![key, version],
            )?;
        }

        Ok(())
    }

    fn search_like(
        &self,
        query: &str,
        favorites_only: bool,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<ClipItem>, StorageError> {
        let pattern = like_pattern(query);
        let sql = if favorites_only {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             WHERE is_favorite = 1
               AND (text_content LIKE ?1 ESCAPE '\\' OR ocr_text LIKE ?1 ESCAPE '\\')
             ORDER BY created_at DESC
             LIMIT ?2 OFFSET ?3"
        } else {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             WHERE text_content LIKE ?1 ESCAPE '\\' OR ocr_text LIKE ?1 ESCAPE '\\'
             ORDER BY created_at DESC
             LIMIT ?2 OFFSET ?3"
        };

        let mut stmt = self.conn.prepare_cached(sql)?;
        let clips = stmt
            .query_map(params![pattern, limit, offset], row_to_clip)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(clips)
    }

    fn search_fts_like(
        &self,
        fts_query: &str,
        query: &str,
        favorites_only: bool,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<ClipItem>, StorageError> {
        let pattern = like_pattern(query);
        let sql = if favorites_only {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             WHERE is_favorite = 1
               AND (
                    id IN (SELECT rowid FROM clips_fts WHERE clips_fts MATCH ?1)
                    OR text_content LIKE ?2 ESCAPE '\\'
                    OR ocr_text LIKE ?2 ESCAPE '\\'
               )
             ORDER BY created_at DESC
             LIMIT ?3 OFFSET ?4"
        } else {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             WHERE id IN (SELECT rowid FROM clips_fts WHERE clips_fts MATCH ?1)
                OR text_content LIKE ?2 ESCAPE '\\'
                OR ocr_text LIKE ?2 ESCAPE '\\'
             ORDER BY created_at DESC
             LIMIT ?3 OFFSET ?4"
        };

        let mut stmt = self.conn.prepare_cached(sql)?;
        let clips = stmt
            .query_map(params![fts_query, pattern, limit, offset], row_to_clip)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(clips)
    }

    /// 插入新条目。若 content_hash 已存在则更新 created_at 并返回该条目。
    /// 用事务包装 clips INSERT + FTS INSERT：两条语句之间失败会让 FTS 缺一行，
    /// 而 `rebuild_fts_once` 只在 schema 版本变化时跑，索引不会自己长回来，
    /// 那条剪贴板记录就永远搜不到。
    #[allow(clippy::too_many_arguments)]
    pub fn insert_clip(
        &self,
        content_type: &ContentType,
        text_content: Option<&str>,
        html_content: Option<&str>,
        image_data: Option<&[u8]>,
        content_hash: &str,
        byte_size: i64,
        is_sensitive: bool,
    ) -> Result<ClipItem, StorageError> {
        let now = now_secs();

        // unchecked_transaction 而不是 transaction()：StorageEngine 只持有 &self，
        // 拿不到 &mut Connection（外层已经被 Arc<Mutex<_>> 串行化，没有并发嵌套）。
        let tx = self.conn.unchecked_transaction()?;

        // UPSERT：新插入或哈希重复时更新 created_at 置顶
        self.conn.execute(
            "INSERT INTO clips
                (content_type, text_content, html_content, image_data, content_hash, is_favorite, created_at, byte_size, is_sensitive)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8)
             ON CONFLICT(content_hash) DO UPDATE SET created_at = excluded.created_at",
            params![
                content_type.as_str(),
                text_content,
                html_content,
                image_data,
                content_hash,
                now,
                byte_size,
                is_sensitive as i64,
            ],
        )?;

        let id: i64 = self.conn.query_row(
            "SELECT id FROM clips WHERE content_hash = ?1",
            params![content_hash],
            |row| row.get(0),
        )?;

        // 新插入时同步 FTS 索引（last_insert_rowid 仅在真正 INSERT 时更新为新行 id）
        if self.conn.last_insert_rowid() == id {
            self.conn.execute(
                "INSERT OR IGNORE INTO clips_fts(rowid, text_content) VALUES (?1, ?2)",
                params![id, text_content],
            )?;
        }

        tx.commit()?;
        self.get_clip_by_id(id)
    }

    /// 通过 id 获取单条记录
    pub fn get_clip_by_id(&self, id: i64) -> Result<ClipItem, StorageError> {
        let clip = self.conn.query_row(
            "SELECT id, content_type, text_content, html_content, image_data,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips WHERE id = ?1",
            params![id],
            row_to_clip,
        )?;
        Ok(clip)
    }

    /// 更新指定条目的 created_at 为当前时间（用于 select_clip 置顶），并返回更新后的条目
    pub fn touch_clip(&self, id: i64) -> Result<ClipItem, StorageError> {
        let now = now_secs();
        self.conn.execute(
            "UPDATE clips SET created_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        self.get_clip_by_id(id)
    }

    /// 通过 id 获取图片二进制数据（仅 image 类型有值）
    pub fn get_clip_image(&self, id: i64) -> Result<Option<Vec<u8>>, StorageError> {
        let result = self.conn.query_row(
            "SELECT image_data FROM clips WHERE id = ?1",
            params![id],
            |row| row.get(0),
        );
        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Database(e)),
        }
    }

    /// 读取缓存的 OCR 文字
    pub fn get_ocr_text(&self, id: i64) -> Result<Option<String>, StorageError> {
        let result = self.conn.query_row(
            "SELECT ocr_text FROM clips WHERE id = ?1",
            params![id],
            |row| row.get(0),
        );
        match result {
            Ok(text) => Ok(text),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Database(e)),
        }
    }

    /// 保存 OCR 识别结果
    pub fn set_ocr_text(&self, id: i64, text: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "UPDATE clips SET ocr_text = ?1 WHERE id = ?2",
            params![text, id],
        )?;
        Ok(())
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
            let sanitized = sanitize_search_query(trimmed);
            if sanitized.is_empty() {
                return Ok(Vec::new());
            }

            if sanitized.chars().count() < 3 {
                return self.search_like(&sanitized, favorites_only, offset, limit);
            }

            if let Some(fts_query) = build_fts_prefix_query(&sanitized) {
                match self.search_fts_like(&fts_query, &sanitized, favorites_only, offset, limit) {
                    Ok(clips) => return Ok(clips),
                    Err(StorageError::Database(rusqlite::Error::SqliteFailure(_, _))) => {
                        return self.search_like(&sanitized, favorites_only, offset, limit);
                    }
                    Err(e) => return Err(e),
                }
            }

            return self.search_like(&sanitized, favorites_only, offset, limit);
        }

        // 普通查询路径
        let sql = if favorites_only {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             WHERE is_favorite = 1
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2"
        } else {
            "SELECT id, content_type, text_content, NULL, NULL,
                    content_hash, is_favorite, created_at, byte_size, is_sensitive
             FROM clips
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2"
        };

        let mut stmt = self.conn.prepare_cached(sql)?;
        let clips = stmt
            .query_map(params![limit, offset], row_to_clip)?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(clips)
    }

    /// 删除指定条目（先清理 FTS 索引，再删主表、再删译文）。
    ///
    /// 条目不存在时直接返回 Ok 而非静默执行无效操作。
    /// 三条 DELETE 必须同生共死：中途失败要么留下搜得到的幽灵 FTS 行，
    /// 要么把译文留在 translation_history 里——"删条目会一并删掉它的译文"
    /// 是对用户承诺的隐私不变量，不能因为一次 SQLite 出错就破功。
    pub fn delete_clip(&self, id: i64) -> Result<(), StorageError> {
        let tx = self.conn.unchecked_transaction()?;

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
        // 条目的译文同样是它的内容，一并删除。
        self.conn.execute(
            "DELETE FROM translation_history WHERE clip_id = ?1",
            params![id],
        )?;
        tx.commit()?;
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
        let tx = self.conn.unchecked_transaction()?;
        self.conn
            .execute("DELETE FROM clips WHERE is_favorite = 0", [])?;
        // 重建 FTS 虚拟表
        self.conn
            .execute("INSERT INTO clips_fts(clips_fts) VALUES ('rebuild')", [])?;
        self.purge_orphan_translations()?;
        tx.commit()?;
        Ok(())
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
        is_sensitive: {
            let v: i64 = row.get(9).unwrap_or(0);
            v != 0
        },
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

#[cfg(test)]
mod tests;
