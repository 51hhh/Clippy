use super::{now_secs, StorageEngine, StorageError};
use crate::models::TranslationHistoryEntry;
use rusqlite::{params, Result as SqlResult};
use sha2::{Digest, Sha256};

/// 全库保留的翻译记录条数上限。译文比剪贴板条目小得多，但同样不该无限增长。
const MAX_TRANSLATION_HISTORY: i64 = 500;

/// 待写入的一条翻译记录。`clip_id` 为 None 表示不来自剪贴板条目
/// （选区翻译或临时文本），落库时存 0。
pub struct NewTranslation<'a> {
    pub clip_id: Option<i64>,
    pub provider: &'a str,
    pub source_language: &'a str,
    pub target_language: &'a str,
    pub source_text: &'a str,
    pub translated_text: &'a str,
}

fn source_hash(text: &str) -> String {
    format!("{:x}", Sha256::new_with_prefix(text.as_bytes()).finalize())
}

impl StorageEngine {
    /// 记录一次成功的翻译。同一条目、同一服务、同一目标语言下的同一段原文
    /// 只保留最新一条，重复翻译不会堆积记录。
    pub fn record_translation(&self, entry: &NewTranslation<'_>) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO translation_history
                (clip_id, provider, source_language, target_language,
                 source_hash, source_text, translated_text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(clip_id, provider, target_language, source_hash) DO UPDATE SET
                source_language = excluded.source_language,
                translated_text = excluded.translated_text,
                created_at = excluded.created_at",
            params![
                entry.clip_id.unwrap_or(0),
                entry.provider,
                entry.source_language,
                entry.target_language,
                source_hash(entry.source_text),
                entry.source_text,
                entry.translated_text,
                now_secs(),
            ],
        )?;
        self.prune_translation_history()
    }

    /// 翻译记录，最新的在前。`clip_id` 为 Some 时只返回该条目的记录。
    pub fn translation_history(
        &self,
        clip_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<TranslationHistoryEntry>, StorageError> {
        let limit = limit.clamp(1, MAX_TRANSLATION_HISTORY);
        let sql = "SELECT id, clip_id, provider, source_language, target_language,
                          source_text, translated_text, created_at
                   FROM translation_history";
        let order = "ORDER BY created_at DESC, id DESC LIMIT ?";
        match clip_id {
            Some(clip_id) => {
                let mut stmt = self
                    .conn
                    .prepare_cached(&format!("{sql} WHERE clip_id = ? {order}"))?;
                let entries = stmt
                    .query_map(params![clip_id, limit], row_to_translation)?
                    .collect::<SqlResult<Vec<_>>>()?;
                Ok(entries)
            }
            None => {
                let mut stmt = self.conn.prepare_cached(&format!("{sql} {order}"))?;
                let entries = stmt
                    .query_map(params![limit], row_to_translation)?
                    .collect::<SqlResult<Vec<_>>>()?;
                Ok(entries)
            }
        }
    }

    /// 清空全部翻译记录。译文一旦落盘，用户必须有办法把它删掉。
    pub fn clear_translation_history(&self) -> Result<(), StorageError> {
        self.conn.execute("DELETE FROM translation_history", [])?;
        Ok(())
    }

    /// 删除已不存在的剪贴板条目留下的记录。条目删除后它的译文不该继续留在库里。
    pub(super) fn purge_orphan_translations(&self) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM translation_history
             WHERE clip_id <> 0 AND clip_id NOT IN (SELECT id FROM clips)",
            [],
        )?;
        Ok(())
    }

    fn prune_translation_history(&self) -> Result<(), StorageError> {
        self.conn.execute(
            "DELETE FROM translation_history WHERE id NOT IN (
                 SELECT id FROM translation_history ORDER BY created_at DESC, id DESC LIMIT ?1
             )",
            params![MAX_TRANSLATION_HISTORY],
        )?;
        Ok(())
    }
}

fn row_to_translation(row: &rusqlite::Row<'_>) -> SqlResult<TranslationHistoryEntry> {
    Ok(TranslationHistoryEntry {
        id: row.get(0)?,
        clip_id: row.get(1)?,
        provider: row.get(2)?,
        source_language: row.get(3)?,
        target_language: row.get(4)?,
        source_text: row.get(5)?,
        translated_text: row.get(6)?,
        created_at: row.get(7)?,
    })
}
