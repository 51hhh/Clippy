use super::{now_secs, StorageEngine, StorageError, StoredPinPlacement};
use crate::models::ContentType;
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchiveScope {
    Full,
    Favorites,
    PinWorkspace,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArchiveRevisionSnapshot {
    pub source_png: Vec<u8>,
    pub source_hash: String,
    pub source_width: u32,
    pub source_height: u32,
    pub renderer_version: u32,
    pub annotations_json: String,
    pub adjustments_json: String,
    pub document_hash: String,
    pub rendered_hash: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArchiveClipSnapshot {
    pub content_type: ContentType,
    pub text_content: Option<String>,
    pub html_content: Option<String>,
    pub image_png: Option<Vec<u8>>,
    pub content_hash: String,
    pub ocr_text: Option<String>,
    pub is_favorite: bool,
    pub is_sensitive: bool,
    pub created_at: i64,
    pub byte_size: i64,
    pub revision: Option<ArchiveRevisionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveGroupSnapshot {
    pub source_id: i64,
    pub name: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ArchiveWorkspaceContentSnapshot {
    Image {
        revision: ArchiveRevisionSnapshot,
    },
    Snapshot {
        content_type: ContentType,
        text_content: Option<String>,
        html_content: Option<String>,
        content_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArchiveWorkspaceSnapshot {
    pub source_id: i64,
    pub group_source_id: Option<i64>,
    pub content: ArchiveWorkspaceContentSnapshot,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub above: bool,
    pub placement: Option<StoredPinPlacement>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArchiveSnapshot {
    pub scope: ArchiveScope,
    pub clips: Vec<ArchiveClipSnapshot>,
    pub groups: Vec<ArchiveGroupSnapshot>,
    pub workspaces: Vec<ArchiveWorkspaceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportSummary {
    pub clips_added: usize,
    pub clips_merged: usize,
    pub groups_added: usize,
    pub workspaces_added: usize,
}

impl StorageEngine {
    pub(crate) fn export_archive_snapshot(
        &self,
        scope: ArchiveScope,
        include_sensitive: bool,
    ) -> Result<ArchiveSnapshot, StorageError> {
        let clips = if scope == ArchiveScope::PinWorkspace {
            Vec::new()
        } else {
            self.archive_clips(scope == ArchiveScope::Favorites, include_sensitive)?
        };
        let (groups, workspaces) = if scope == ArchiveScope::Favorites {
            (Vec::new(), Vec::new())
        } else {
            (self.archive_groups()?, self.archive_workspaces()?)
        };
        Ok(ArchiveSnapshot {
            scope,
            clips,
            groups,
            workspaces,
        })
    }

    fn archive_clips(
        &self,
        favorites_only: bool,
        include_sensitive: bool,
    ) -> Result<Vec<ArchiveClipSnapshot>, StorageError> {
        let mut statement = self.conn.prepare(
            "SELECT c.id, c.content_type, c.text_content, c.html_content,
                    COALESCE(c.image_data, a.png), c.content_hash, c.ocr_text,
                    c.is_favorite, c.is_sensitive, c.created_at, c.byte_size
               FROM clips c LEFT JOIN image_assets a ON a.id = c.image_asset_id
              WHERE (?1 = 0 OR c.is_favorite = 1)
                AND (?2 = 1 OR c.is_sensitive = 0)
              ORDER BY c.use_order ASC, c.id ASC",
        )?;
        let rows = statement.query_map(
            params![i64::from(favorites_only), i64::from(include_sensitive)],
            |row| {
                let raw_type: String = row.get(1)?;
                let content_type = raw_type.parse::<ContentType>().map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
                    )
                })?;
                Ok((
                    row.get::<_, i64>(0)?,
                    content_type,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            },
        )?;
        let mut clips = Vec::new();
        for row in rows {
            let (
                id,
                content_type,
                text_content,
                html_content,
                image_png,
                content_hash,
                ocr_text,
                is_favorite,
                is_sensitive,
                created_at,
                byte_size,
            ) = row?;
            let revision = self.archive_revision_for_clip(id)?;
            clips.push(ArchiveClipSnapshot {
                content_type,
                text_content,
                html_content,
                image_png,
                content_hash,
                ocr_text,
                is_favorite,
                is_sensitive,
                created_at,
                byte_size,
                revision,
            });
        }
        Ok(clips)
    }

    fn archive_groups(&self) -> Result<Vec<ArchiveGroupSnapshot>, StorageError> {
        Ok(self
            .list_pin_groups()?
            .into_iter()
            .map(|group| ArchiveGroupSnapshot {
                source_id: group.id,
                name: group.name,
                sort_order: group.sort_order,
            })
            .collect())
    }

    fn archive_workspaces(&self) -> Result<Vec<ArchiveWorkspaceSnapshot>, StorageError> {
        self.load_pin_workspace_items()?
            .into_iter()
            .map(|item| {
                let content = match item.content {
                    super::StoredPinWorkspaceContent::Image { revision_id, .. } => {
                        ArchiveWorkspaceContentSnapshot::Image {
                            revision: self.archive_revision_by_id(revision_id)?.ok_or_else(
                                || {
                                    StorageError::Invariant(
                                        "Pin 工作区引用的图片修订不存在".to_string(),
                                    )
                                },
                            )?,
                        }
                    }
                    super::StoredPinWorkspaceContent::Snapshot {
                        content_type,
                        text_content,
                        html_content,
                        content_hash,
                    } => ArchiveWorkspaceContentSnapshot::Snapshot {
                        content_type,
                        text_content,
                        html_content,
                        content_hash,
                    },
                };
                Ok(ArchiveWorkspaceSnapshot {
                    source_id: item.id,
                    group_source_id: item.group_id,
                    content,
                    content_width: item.content_width,
                    content_height: item.content_height,
                    scale: item.scale,
                    opacity: item.opacity,
                    locked: item.locked,
                    above: item.above,
                    placement: item.placement,
                })
            })
            .collect()
    }

    fn archive_revision_for_clip(
        &self,
        clip_id: i64,
    ) -> Result<Option<ArchiveRevisionSnapshot>, StorageError> {
        let revision_id = self
            .conn
            .query_row(
                "SELECT revision_id FROM clip_image_revisions WHERE clip_id = ?1",
                [clip_id],
                |row| row.get(0),
            )
            .optional()?;
        revision_id
            .map(|id| self.archive_revision_by_id(id))
            .transpose()
            .map(Option::flatten)
    }

    fn archive_revision_by_id(
        &self,
        revision_id: i64,
    ) -> Result<Option<ArchiveRevisionSnapshot>, StorageError> {
        let row = self
            .conn
            .query_row(
                "SELECT a.png, a.source_hash, r.source_width, r.source_height,
                        r.renderer_version, r.annotations_json, r.adjustments_json,
                        r.document_hash, r.rendered_hash
                   FROM image_revisions r JOIN image_assets a ON a.id = r.asset_id
                  WHERE r.id = ?1",
                [revision_id],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, u32>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(
                source_png,
                source_hash,
                source_width,
                source_height,
                renderer_version,
                annotations,
                adjustments,
                document_hash,
                rendered_hash,
            )| {
                Ok(ArchiveRevisionSnapshot {
                    source_png,
                    source_hash,
                    source_width,
                    source_height,
                    renderer_version,
                    annotations_json: annotations,
                    adjustments_json: adjustments,
                    document_hash,
                    rendered_hash,
                })
            },
        )
        .transpose()
    }

    pub(crate) fn import_archive_snapshot(
        &self,
        snapshot: &ArchiveSnapshot,
    ) -> Result<ImportSummary, StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut summary = ImportSummary {
            clips_added: 0,
            clips_merged: 0,
            groups_added: 0,
            workspaces_added: 0,
        };
        for clip in &snapshot.clips {
            let existing = self
                .conn
                .query_row(
                    "SELECT id, content_type FROM clips WHERE content_hash = ?1",
                    [&clip.content_hash],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let clip_id = if let Some((id, content_type)) = existing {
                if content_type != clip.content_type.as_str() {
                    return Err(StorageError::Invariant(
                        "归档内容哈希与现有条目类型冲突".to_string(),
                    ));
                }
                self.conn.execute(
                    "UPDATE clips SET
                        is_favorite = MAX(is_favorite, ?1),
                        is_sensitive = MAX(is_sensitive, ?2),
                        ocr_text = COALESCE(ocr_text, ?3)
                      WHERE id = ?4",
                    params![
                        i64::from(clip.is_favorite),
                        i64::from(clip.is_sensitive),
                        clip.ocr_text,
                        id
                    ],
                )?;
                summary.clips_merged += 1;
                id
            } else {
                let use_order = self.next_use_order()?;
                self.conn.execute(
                    "INSERT INTO clips(
                        content_type, text_content, html_content, image_data, content_hash,
                        is_favorite, created_at, byte_size, is_sensitive, use_order, ocr_text)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        clip.content_type.as_str(),
                        clip.text_content,
                        clip.html_content,
                        clip.image_png,
                        clip.content_hash,
                        i64::from(clip.is_favorite),
                        clip.created_at,
                        clip.byte_size,
                        i64::from(clip.is_sensitive),
                        use_order,
                        clip.ocr_text,
                    ],
                )?;
                let id = self.conn.last_insert_rowid();
                self.conn.execute(
                    "INSERT INTO clips_fts(rowid, text_content) VALUES (?1, ?2)",
                    params![id, clip.text_content],
                )?;
                summary.clips_added += 1;
                id
            };
            if let Some(revision) = &clip.revision {
                let revision_id = self.import_archive_revision(revision)?;
                self.conn.execute(
                    "INSERT OR IGNORE INTO clip_image_revisions(clip_id, revision_id)
                     VALUES (?1, ?2)",
                    params![clip_id, revision_id],
                )?;
                if clip.content_hash == revision.source_hash
                    && clip.image_png.as_deref() == Some(revision.source_png.as_slice())
                {
                    self.conn.execute(
                        "UPDATE clips SET
                            image_asset_id = (SELECT asset_id FROM image_revisions WHERE id = ?1),
                            image_data = NULL
                          WHERE id = ?2 AND content_type = 'image' AND content_hash = ?3",
                        params![revision_id, clip_id, clip.content_hash],
                    )?;
                }
            }
        }

        let mut group_ids = HashMap::new();
        let mut groups = snapshot.groups.iter().collect::<Vec<_>>();
        groups.sort_by_key(|group| (group.sort_order, group.source_id));
        let mut next_group_order: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM pin_groups",
            [],
            |row| row.get(0),
        )?;
        for group in groups {
            let existing = self
                .conn
                .query_row(
                    "SELECT id FROM pin_groups WHERE name = ?1 COLLATE NOCASE",
                    [&group.name],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            let id = if let Some(id) = existing {
                id
            } else {
                self.conn.execute(
                    "INSERT INTO pin_groups(name, sort_order, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![group.name, next_group_order, now_secs()],
                )?;
                summary.groups_added += 1;
                let id = self.conn.last_insert_rowid();
                next_group_order = next_group_order
                    .checked_add(1)
                    .ok_or_else(|| StorageError::Invariant("Pin 分组排序值溢出".to_string()))?;
                id
            };
            group_ids.insert(group.source_id, id);
        }

        for workspace in &snapshot.workspaces {
            let group_id = workspace
                .group_source_id
                .map(|id| {
                    group_ids.get(&id).copied().ok_or_else(|| {
                        StorageError::PinWorkspaceInvariant(
                            "归档工作区引用了不存在的分组".to_string(),
                        )
                    })
                })
                .transpose()?;
            let (revision_id, content_type, text, html, content_hash) = match &workspace.content {
                ArchiveWorkspaceContentSnapshot::Image { revision } => (
                    Some(self.import_archive_revision(revision)?),
                    ContentType::Image,
                    None,
                    None,
                    revision.rendered_hash.as_str(),
                ),
                ArchiveWorkspaceContentSnapshot::Snapshot {
                    content_type,
                    text_content,
                    html_content,
                    content_hash,
                } => (
                    None,
                    content_type.clone(),
                    text_content.as_deref(),
                    html_content.as_deref(),
                    content_hash.as_str(),
                ),
            };
            let placement = workspace.placement.as_ref();
            self.conn.execute(
                "INSERT INTO pin_workspace_items(
                    group_id, revision_id, content_type, text_content, html_content, content_hash,
                    content_width, content_height, scale, opacity, locked, above,
                    position_x, position_y, display_name, display_x, display_y,
                    display_width, display_height, display_scale, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?21)",
                params![
                    group_id,
                    revision_id,
                    content_type.as_str(),
                    text,
                    html,
                    content_hash,
                    workspace.content_width,
                    workspace.content_height,
                    workspace.scale,
                    workspace.opacity,
                    i64::from(workspace.locked),
                    i64::from(workspace.above),
                    placement.map(|value| value.x),
                    placement.map(|value| value.y),
                    placement.and_then(|value| value.display_name.as_deref()),
                    placement.map(|value| value.display_x),
                    placement.map(|value| value.display_y),
                    placement.map(|value| value.display_width),
                    placement.map(|value| value.display_height),
                    placement.map(|value| value.display_scale),
                    now_secs(),
                ],
            )?;
            summary.workspaces_added += 1;
        }
        tx.commit()?;
        Ok(summary)
    }

    fn import_archive_revision(
        &self,
        revision: &ArchiveRevisionSnapshot,
    ) -> Result<i64, StorageError> {
        let byte_size = i64::try_from(revision.source_png.len())
            .map_err(|error| StorageError::Invariant(error.to_string()))?;
        self.conn.execute(
            "INSERT OR IGNORE INTO image_assets(source_hash, png, width, height, byte_size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                revision.source_hash,
                revision.source_png,
                revision.source_width,
                revision.source_height,
                byte_size,
                now_secs(),
            ],
        )?;
        let (asset_id, exact): (i64, bool) = self.conn.query_row(
            "SELECT id, png = ?2 AND width = ?3 AND height = ?4
               FROM image_assets WHERE source_hash = ?1",
            params![
                revision.source_hash,
                revision.source_png,
                revision.source_width,
                revision.source_height,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if !exact {
            return Err(StorageError::Invariant(
                "归档根图哈希与已有根图冲突".to_string(),
            ));
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO image_revisions(
                asset_id, renderer_version, source_width, source_height,
                annotations_json, adjustments_json, document_hash, rendered_hash, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                asset_id,
                revision.renderer_version,
                revision.source_width,
                revision.source_height,
                revision.annotations_json,
                revision.adjustments_json,
                revision.document_hash,
                revision.rendered_hash,
                now_secs(),
            ],
        )?;
        let (id, rendered_hash): (i64, String) = self.conn.query_row(
            "SELECT id, rendered_hash FROM image_revisions
              WHERE asset_id = ?1 AND document_hash = ?2",
            params![asset_id, revision.document_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if rendered_hash != revision.rendered_hash {
            return Err(StorageError::Invariant(
                "归档修订文档与已有渲染哈希冲突".to_string(),
            ));
        }
        Ok(id)
    }
}
