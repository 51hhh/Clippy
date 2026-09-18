use super::{now_secs, StorageEngine, StorageError};
use rusqlite::{params, OptionalExtension};

/// 已验证的修订写入合同。PNG 与 JSON 的尺寸/结构校验由 Pin 权威渲染层完成，
/// 存储层负责在一个 SQLite 事务里去重根图、冻结文档并建立现有 clip 的关联。
pub(crate) struct ImageRevisionWrite<'a> {
    pub source_png: &'a [u8],
    pub source_hash: &'a str,
    pub source_width: u32,
    pub source_height: u32,
    pub renderer_version: u32,
    pub annotations_json: &'a str,
    pub adjustments_json: &'a str,
    pub document_hash: &'a str,
    pub rendered_hash: &'a str,
    /// 普通历史图片第一次成为根图时，把 BLOB 移入 asset，clip 改为引用它，避免存两份。
    pub root_clip_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredImageRevision {
    pub source_png: Vec<u8>,
    pub source_width: u32,
    pub source_height: u32,
    pub renderer_version: u32,
    pub annotations: serde_json::Value,
    pub adjustments: serde_json::Value,
}

impl StorageEngine {
    pub(crate) fn register_image_revision(
        &self,
        write: ImageRevisionWrite<'_>,
    ) -> Result<i64, StorageError> {
        validate_hash(write.source_hash)?;
        validate_hash(write.document_hash)?;
        validate_hash(write.rendered_hash)?;
        let document_bytes = write
            .annotations_json
            .len()
            .checked_add(write.adjustments_json.len())
            .ok_or_else(|| StorageError::Invariant("修订文档长度溢出".to_string()))?;
        if write.source_png.is_empty()
            || write.source_width == 0
            || write.source_height == 0
            || document_bytes > 96 * 1024 * 1024
        {
            return Err(StorageError::Invariant(
                "根图尺寸或修订文档超过约束".to_string(),
            ));
        }
        let byte_size = i64::try_from(write.source_png.len())
            .map_err(|error| StorageError::Invariant(error.to_string()))?;
        let tx = self.conn.unchecked_transaction()?;
        self.conn.execute(
            "INSERT OR IGNORE INTO image_assets
                (source_hash, png, width, height, byte_size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                write.source_hash,
                write.source_png,
                i64::from(write.source_width),
                i64::from(write.source_height),
                byte_size,
                now_secs(),
            ],
        )?;
        let (asset_id, same_asset): (i64, bool) = self.conn.query_row(
            "SELECT id, png = ?2 AND width = ?3 AND height = ?4
               FROM image_assets WHERE source_hash = ?1",
            params![
                write.source_hash,
                write.source_png,
                i64::from(write.source_width),
                i64::from(write.source_height),
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !same_asset {
            return Err(StorageError::Invariant(
                "根图哈希已存在但字节或尺寸不一致".to_string(),
            ));
        }

        if let Some(clip_id) = write.root_clip_id {
            let changed = self.conn.execute(
                "UPDATE clips
                    SET image_asset_id = ?1, image_data = NULL
                  WHERE id = ?2 AND content_type = 'image' AND content_hash = ?3
                    AND (image_asset_id = ?1 OR image_data = ?4)",
                params![asset_id, clip_id, write.source_hash, write.source_png],
            )?;
            if changed != 1 {
                return Err(StorageError::Invariant(
                    "根图片段已改变，拒绝建立修订指针".to_string(),
                ));
            }
        }

        self.conn.execute(
            "INSERT OR IGNORE INTO image_revisions
                (asset_id, renderer_version, source_width, source_height,
                 annotations_json, adjustments_json, document_hash, rendered_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                asset_id,
                i64::from(write.renderer_version),
                i64::from(write.source_width),
                i64::from(write.source_height),
                write.annotations_json,
                write.adjustments_json,
                write.document_hash,
                write.rendered_hash,
                now_secs(),
            ],
        )?;
        let (revision_id, rendered_hash): (i64, String) = self.conn.query_row(
            "SELECT id, rendered_hash FROM image_revisions
              WHERE asset_id = ?1 AND document_hash = ?2",
            params![asset_id, write.document_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if rendered_hash != write.rendered_hash {
            return Err(StorageError::Invariant(
                "同一根图与文档产生了不同渲染结果".to_string(),
            ));
        }
        // 同一像素可能由多个文档生成。已有 clip 关联一旦建立就保持稳定；没有关联时
        // 才选当前修订，避免后来保存的等像素文档悄悄改变旧历史的编辑语义。
        self.conn.execute(
            "INSERT OR IGNORE INTO clip_image_revisions(clip_id, revision_id)
             SELECT id, ?2 FROM clips WHERE content_type = 'image' AND content_hash = ?1",
            params![write.rendered_hash, revision_id],
        )?;
        tx.commit()?;
        Ok(revision_id)
    }

    pub(crate) fn get_image_revision_for_clip(
        &self,
        clip_id: i64,
    ) -> Result<Option<StoredImageRevision>, StorageError> {
        let row = self
            .conn
            .query_row(
                "SELECT a.png, r.source_width, r.source_height, r.renderer_version,
                        r.annotations_json, r.adjustments_json
                   FROM clip_image_revisions l
                   JOIN image_revisions r ON r.id = l.revision_id
                   JOIN image_assets a ON a.id = r.asset_id
                  WHERE l.clip_id = ?1",
                params![clip_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            source_png,
            source_width,
            source_height,
            renderer_version,
            annotations,
            adjustments,
        )) = row
        else {
            return Ok(None);
        };
        let annotations = serde_json::from_str(&annotations)
            .map_err(|error| StorageError::Invariant(format!("标注 JSON 损坏: {error}")))?;
        let adjustments = serde_json::from_str(&adjustments)
            .map_err(|error| StorageError::Invariant(format!("调整 JSON 损坏: {error}")))?;
        Ok(Some(StoredImageRevision {
            source_png,
            source_width,
            source_height,
            renderer_version,
            annotations,
            adjustments,
        }))
    }

    pub(crate) fn get_image_revision_by_rendered_hash(
        &self,
        rendered_hash: &str,
    ) -> Result<Option<StoredImageRevision>, StorageError> {
        validate_hash(rendered_hash)?;
        let revision_id = self
            .conn
            .query_row(
                "SELECT id FROM image_revisions WHERE rendered_hash = ?1 ORDER BY id DESC LIMIT 1",
                params![rendered_hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(revision_id) = revision_id else {
            return Ok(None);
        };
        let (source_png, source_width, source_height, renderer_version, annotations, adjustments) =
            self.conn.query_row(
                "SELECT a.png, r.source_width, r.source_height, r.renderer_version,
                        r.annotations_json, r.adjustments_json
                   FROM image_revisions r JOIN image_assets a ON a.id = r.asset_id
                  WHERE r.id = ?1",
                params![revision_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )?;
        Ok(Some(StoredImageRevision {
            source_png,
            source_width,
            source_height,
            renderer_version,
            annotations: serde_json::from_str(&annotations)
                .map_err(|error| StorageError::Invariant(format!("标注 JSON 损坏: {error}")))?,
            adjustments: serde_json::from_str(&adjustments)
                .map_err(|error| StorageError::Invariant(format!("调整 JSON 损坏: {error}")))?,
        }))
    }

    pub(super) fn link_clip_revision_for_hash(
        &self,
        clip_id: i64,
        rendered_hash: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO clip_image_revisions(clip_id, revision_id)
             SELECT ?1, id FROM image_revisions
              WHERE rendered_hash = ?2
              ORDER BY id DESC LIMIT 1",
            params![clip_id, rendered_hash],
        )?;
        Ok(())
    }
}

fn validate_hash(hash: &str) -> Result<(), StorageError> {
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(StorageError::Invariant("修订哈希无效".to_string()))
    }
}
