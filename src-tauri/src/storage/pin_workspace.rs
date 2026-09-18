use super::{now_secs, StorageEngine, StorageError};
use crate::models::ContentType;
use rusqlite::params;

const MAX_GROUP_NAME_CHARS: usize = 64;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinWorkspaceGroup {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredPinPlacement {
    /// 窗口外框左上角，桌面全局逻辑像素。
    pub x: f64,
    pub y: f64,
    pub display_name: Option<String>,
    /// 保存时显示器工作区，桌面全局逻辑像素。
    pub display_x: f64,
    pub display_y: f64,
    pub display_width: f64,
    pub display_height: f64,
    pub display_scale: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StoredPinWorkspaceContent {
    Image {
        revision_id: i64,
        revision: super::StoredImageRevision,
    },
    Snapshot {
        content_type: ContentType,
        text_content: Option<String>,
        html_content: Option<String>,
        content_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredPinWorkspaceItem {
    pub id: i64,
    pub group_id: Option<i64>,
    pub content: StoredPinWorkspaceContent,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub above: bool,
    pub placement: Option<StoredPinPlacement>,
}

pub(crate) struct PinWorkspaceItemWrite<'a> {
    pub id: Option<i64>,
    pub group_id: Option<i64>,
    pub revision_id: Option<i64>,
    pub content_type: ContentType,
    pub text_content: Option<&'a str>,
    pub html_content: Option<&'a str>,
    pub content_hash: &'a str,
    pub content_width: f64,
    pub content_height: f64,
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub above: bool,
    pub placement: Option<&'a StoredPinPlacement>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PinWorkspacePresentation {
    pub scale: f64,
    pub opacity: f64,
    pub locked: bool,
    pub above: bool,
    pub placement: Option<StoredPinPlacement>,
}

impl StorageEngine {
    pub(super) fn migrate_pin_workspace(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pin_groups (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL COLLATE NOCASE UNIQUE,
                sort_order INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS pin_workspace_items (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id       INTEGER REFERENCES pin_groups(id) ON DELETE SET NULL,
                revision_id    INTEGER REFERENCES image_revisions(id) ON DELETE RESTRICT,
                content_type   TEXT NOT NULL,
                text_content   TEXT,
                html_content   TEXT,
                content_hash   TEXT NOT NULL,
                content_width  REAL NOT NULL,
                content_height REAL NOT NULL,
                scale          REAL NOT NULL,
                opacity        REAL NOT NULL,
                locked         INTEGER NOT NULL,
                above          INTEGER NOT NULL,
                position_x     REAL,
                position_y     REAL,
                display_name   TEXT,
                display_x      REAL,
                display_y      REAL,
                display_width  REAL,
                display_height REAL,
                display_scale  REAL,
                created_at     INTEGER NOT NULL,
                updated_at     INTEGER NOT NULL,
                CHECK(content_type IN ('text', 'html', 'image')),
                CHECK(content_width > 0 AND content_height > 0),
                CHECK(scale >= 0.25 AND scale <= 4.0),
                CHECK(opacity >= 0.15 AND opacity <= 1.0),
                CHECK(locked IN (0, 1) AND above IN (0, 1)),
                CHECK((content_type = 'image' AND revision_id IS NOT NULL
                       AND text_content IS NULL AND html_content IS NULL)
                   OR (content_type != 'image' AND revision_id IS NULL))
             );
             CREATE INDEX IF NOT EXISTS idx_pin_workspace_group
                ON pin_workspace_items(group_id, id);
             CREATE INDEX IF NOT EXISTS idx_pin_workspace_revision
                ON pin_workspace_items(revision_id);",
        )?;
        Ok(())
    }

    pub(crate) fn create_pin_group(&self, name: &str) -> Result<PinWorkspaceGroup, StorageError> {
        let name = validate_group_name(name)?;
        let sort_order: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM pin_groups",
            [],
            |row| row.get(0),
        )?;
        let now = now_secs();
        self.conn.execute(
            "INSERT INTO pin_groups(name, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            params![name, sort_order, now],
        )?;
        Ok(PinWorkspaceGroup {
            id: self.conn.last_insert_rowid(),
            name,
            sort_order,
        })
    }

    pub(crate) fn rename_pin_group(&self, id: i64, name: &str) -> Result<bool, StorageError> {
        let name = validate_group_name(name)?;
        Ok(self.conn.execute(
            "UPDATE pin_groups SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now_secs(), id],
        )? == 1)
    }

    pub(crate) fn delete_pin_group(&self, id: i64) -> Result<bool, StorageError> {
        Ok(self
            .conn
            .execute("DELETE FROM pin_groups WHERE id = ?1", [id])?
            == 1)
    }

    pub(crate) fn list_pin_groups(&self) -> Result<Vec<PinWorkspaceGroup>, StorageError> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, sort_order FROM pin_groups ORDER BY sort_order ASC, id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PinWorkspaceGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                sort_order: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn upsert_pin_workspace_item(
        &self,
        write: PinWorkspaceItemWrite<'_>,
    ) -> Result<i64, StorageError> {
        validate_item_write(&write)?;
        let now = now_secs();
        let placement = placement_columns(write.placement);
        let locked = i64::from(write.locked);
        let above = i64::from(write.above);
        if let Some(id) = write.id {
            let changed = self.conn.execute(
                "UPDATE pin_workspace_items SET
                    group_id=?1, revision_id=?2, content_type=?3, text_content=?4,
                    html_content=?5, content_hash=?6, content_width=?7, content_height=?8,
                    scale=?9, opacity=?10, locked=?11, above=?12,
                    position_x=?13, position_y=?14, display_name=?15, display_x=?16,
                    display_y=?17, display_width=?18, display_height=?19, display_scale=?20,
                    updated_at=?21 WHERE id=?22",
                params![
                    write.group_id,
                    write.revision_id,
                    write.content_type.as_str(),
                    write.text_content,
                    write.html_content,
                    write.content_hash,
                    write.content_width,
                    write.content_height,
                    write.scale,
                    write.opacity,
                    locked,
                    above,
                    placement.0,
                    placement.1,
                    placement.2,
                    placement.3,
                    placement.4,
                    placement.5,
                    placement.6,
                    placement.7,
                    now,
                    id,
                ],
            )?;
            if changed != 1 {
                return Err(StorageError::PinWorkspaceInvariant(
                    "要更新的 Pin 工作区记录不存在".to_string(),
                ));
            }
            return Ok(id);
        }

        self.conn.execute(
            "INSERT INTO pin_workspace_items(
                group_id, revision_id, content_type, text_content, html_content, content_hash,
                content_width, content_height, scale, opacity, locked, above,
                position_x, position_y, display_name, display_x, display_y,
                display_width, display_height, display_scale, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?21)",
            params![
                write.group_id,
                write.revision_id,
                write.content_type.as_str(),
                write.text_content,
                write.html_content,
                write.content_hash,
                write.content_width,
                write.content_height,
                write.scale,
                write.opacity,
                locked,
                above,
                placement.0,
                placement.1,
                placement.2,
                placement.3,
                placement.4,
                placement.5,
                placement.6,
                placement.7,
                now,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub(crate) fn delete_pin_workspace_item(&self, id: i64) -> Result<bool, StorageError> {
        Ok(self
            .conn
            .execute("DELETE FROM pin_workspace_items WHERE id = ?1", [id])?
            == 1)
    }

    pub(crate) fn set_pin_workspace_group(
        &self,
        id: i64,
        group_id: Option<i64>,
    ) -> Result<bool, StorageError> {
        Ok(self.conn.execute(
            "UPDATE pin_workspace_items SET group_id=?1, updated_at=?2 WHERE id=?3",
            params![group_id, now_secs(), id],
        )? == 1)
    }

    pub(crate) fn update_pin_workspace_presentation(
        &self,
        id: i64,
        presentation: &PinWorkspacePresentation,
    ) -> Result<bool, StorageError> {
        validate_presentation(presentation)?;
        let changed = if let Some(placement) = presentation.placement.as_ref() {
            let placement = placement_columns(Some(placement));
            self.conn.execute(
                "UPDATE pin_workspace_items SET scale=?1, opacity=?2, locked=?3, above=?4,
                        position_x=?5, position_y=?6, display_name=?7, display_x=?8,
                        display_y=?9, display_width=?10, display_height=?11, display_scale=?12,
                        updated_at=?13 WHERE id=?14",
                params![
                    presentation.scale,
                    presentation.opacity,
                    i64::from(presentation.locked),
                    i64::from(presentation.above),
                    placement.0,
                    placement.1,
                    placement.2,
                    placement.3,
                    placement.4,
                    placement.5,
                    placement.6,
                    placement.7,
                    now_secs(),
                    id,
                ],
            )?
        } else {
            // 合成器在窗口刚映射或显示器切换期间可能暂时不给位置。此时只保存其它状态，
            // 保留最后一份可信位置，不能用“读不到”覆盖成 NULL。
            self.conn.execute(
                "UPDATE pin_workspace_items SET scale=?1, opacity=?2, locked=?3, above=?4,
                        updated_at=?5 WHERE id=?6",
                params![
                    presentation.scale,
                    presentation.opacity,
                    i64::from(presentation.locked),
                    i64::from(presentation.above),
                    now_secs(),
                    id,
                ],
            )?
        };
        Ok(changed == 1)
    }

    pub(crate) fn load_pin_workspace_items(
        &self,
    ) -> Result<Vec<StoredPinWorkspaceItem>, StorageError> {
        let mut statement = self.conn.prepare(
            "SELECT w.id, CAST(w.group_id AS INTEGER), CAST(w.revision_id AS INTEGER),
                    CAST(w.content_type AS TEXT), CAST(w.text_content AS TEXT),
                    CAST(w.html_content AS TEXT), CAST(w.content_hash AS TEXT),
                    CAST(w.content_width AS REAL), CAST(w.content_height AS REAL),
                    CAST(w.scale AS REAL), CAST(w.opacity AS REAL),
                    CAST(w.locked AS INTEGER), CAST(w.above AS INTEGER),
                    CAST(w.position_x AS REAL), CAST(w.position_y AS REAL),
                    CAST(w.display_name AS TEXT), CAST(w.display_x AS REAL),
                    CAST(w.display_y AS REAL), CAST(w.display_width AS REAL),
                    CAST(w.display_height AS REAL), CAST(w.display_scale AS REAL),
                    a.png, CAST(r.source_width AS INTEGER), CAST(r.source_height AS INTEGER),
                    CAST(r.renderer_version AS INTEGER), CAST(r.annotations_json AS TEXT),
                    CAST(r.adjustments_json AS TEXT)
               FROM pin_workspace_items w
               LEFT JOIN image_revisions r ON r.id = w.revision_id
               LEFT JOIN image_assets a ON a.id = r.asset_id
              ORDER BY w.id ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RawWorkspaceRow {
                id: row.get(0)?,
                group_id: row.get(1)?,
                revision_id: row.get(2)?,
                content_type: row.get(3)?,
                text_content: row.get(4)?,
                html_content: row.get(5)?,
                content_hash: row.get(6)?,
                content_width: row.get(7)?,
                content_height: row.get(8)?,
                scale: row.get(9)?,
                opacity: row.get(10)?,
                locked: row.get(11)?,
                above: row.get(12)?,
                position_x: row.get(13)?,
                position_y: row.get(14)?,
                display_name: row.get(15)?,
                display_x: row.get(16)?,
                display_y: row.get(17)?,
                display_width: row.get(18)?,
                display_height: row.get(19)?,
                display_scale: row.get(20)?,
                source_png: row.get(21)?,
                source_width: row.get(22)?,
                source_height: row.get(23)?,
                renderer_version: row.get(24)?,
                annotations_json: row.get(25)?,
                adjustments_json: row.get(26)?,
            })
        })?;

        let mut items = Vec::new();
        for row in rows {
            let raw = row?;
            match raw.try_into_item() {
                Ok(item) => items.push(item),
                Err(reason) => log::warn!("跳过损坏的 Pin 工作区记录 {}: {reason}", raw.id),
            }
        }
        Ok(items)
    }
}

type PlacementColumns<'a> = (
    Option<f64>,
    Option<f64>,
    Option<&'a str>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

fn placement_columns(placement: Option<&StoredPinPlacement>) -> PlacementColumns<'_> {
    match placement {
        Some(placement) => (
            Some(placement.x),
            Some(placement.y),
            placement.display_name.as_deref(),
            Some(placement.display_x),
            Some(placement.display_y),
            Some(placement.display_width),
            Some(placement.display_height),
            Some(placement.display_scale),
        ),
        None => (None, None, None, None, None, None, None, None),
    }
}

fn validate_group_name(name: &str) -> Result<String, StorageError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_GROUP_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(StorageError::PinWorkspaceInvariant(
            "Pin 分组名称无效".to_string(),
        ));
    }
    Ok(name.to_string())
}

fn validate_item_write(write: &PinWorkspaceItemWrite<'_>) -> Result<(), StorageError> {
    let finite = [
        write.content_width,
        write.content_height,
        write.scale,
        write.opacity,
    ]
    .into_iter()
    .all(f64::is_finite);
    if !finite
        || write.content_width <= 0.0
        || write.content_height <= 0.0
        || !(0.25..=4.0).contains(&write.scale)
        || !(0.15..=1.0).contains(&write.opacity)
    {
        return Err(StorageError::PinWorkspaceInvariant(
            "Pin 工作区显示状态无效".to_string(),
        ));
    }
    if let Some(placement) = write.placement {
        validate_placement(placement)?;
    }
    if write
        .text_content
        .map(str::len)
        .unwrap_or_default()
        .saturating_add(write.html_content.map(str::len).unwrap_or_default())
        > MAX_SNAPSHOT_BYTES
    {
        return Err(StorageError::PinWorkspaceInvariant(
            "Pin 工作区文本快照过大".to_string(),
        ));
    }
    match write.content_type {
        ContentType::Image
            if write.revision_id.is_some()
                && write.text_content.is_none()
                && write.html_content.is_none() => {}
        ContentType::Text | ContentType::Html if write.revision_id.is_none() => {}
        _ => {
            return Err(StorageError::PinWorkspaceInvariant(
                "Pin 工作区内容引用不一致".to_string(),
            ));
        }
    }
    if write.content_hash.len() > 128 || write.content_hash.chars().any(char::is_control) {
        return Err(StorageError::PinWorkspaceInvariant(
            "Pin 工作区内容哈希无效".to_string(),
        ));
    }
    Ok(())
}

fn validate_placement(placement: &StoredPinPlacement) -> Result<(), StorageError> {
    if [
        placement.x,
        placement.y,
        placement.display_x,
        placement.display_y,
        placement.display_width,
        placement.display_height,
        placement.display_scale,
    ]
    .into_iter()
    .all(f64::is_finite)
        && placement.display_width > 0.0
        && placement.display_height > 0.0
        && placement.display_scale > 0.0
        && placement
            .display_name
            .as_ref()
            .is_none_or(|name| name.len() <= 256 && !name.chars().any(char::is_control))
    {
        Ok(())
    } else {
        Err(StorageError::PinWorkspaceInvariant(
            "Pin 工作区显示器位置无效".to_string(),
        ))
    }
}

fn validate_presentation(presentation: &PinWorkspacePresentation) -> Result<(), StorageError> {
    if !presentation.scale.is_finite()
        || !presentation.opacity.is_finite()
        || !(0.25..=4.0).contains(&presentation.scale)
        || !(0.15..=1.0).contains(&presentation.opacity)
    {
        return Err(StorageError::PinWorkspaceInvariant(
            "Pin 工作区显示状态无效".to_string(),
        ));
    }
    if let Some(placement) = &presentation.placement {
        validate_placement(placement)?;
    }
    Ok(())
}

struct RawWorkspaceRow {
    id: i64,
    group_id: Option<i64>,
    revision_id: Option<i64>,
    content_type: String,
    text_content: Option<String>,
    html_content: Option<String>,
    content_hash: String,
    content_width: f64,
    content_height: f64,
    scale: f64,
    opacity: f64,
    locked: i64,
    above: i64,
    position_x: Option<f64>,
    position_y: Option<f64>,
    display_name: Option<String>,
    display_x: Option<f64>,
    display_y: Option<f64>,
    display_width: Option<f64>,
    display_height: Option<f64>,
    display_scale: Option<f64>,
    source_png: Option<Vec<u8>>,
    source_width: Option<u32>,
    source_height: Option<u32>,
    renderer_version: Option<u32>,
    annotations_json: Option<String>,
    adjustments_json: Option<String>,
}

impl RawWorkspaceRow {
    fn try_into_item(&self) -> Result<StoredPinWorkspaceItem, String> {
        let content_type = self.content_type.parse::<ContentType>()?;
        if ![
            self.content_width,
            self.content_height,
            self.scale,
            self.opacity,
        ]
        .into_iter()
        .all(f64::is_finite)
            || self.content_width <= 0.0
            || self.content_height <= 0.0
            || !(0.25..=4.0).contains(&self.scale)
            || !(0.15..=1.0).contains(&self.opacity)
            || !matches!(self.locked, 0 | 1)
            || !matches!(self.above, 0 | 1)
        {
            return Err("显示状态无效".to_string());
        }
        let placement = match (
            self.position_x,
            self.position_y,
            self.display_x,
            self.display_y,
            self.display_width,
            self.display_height,
            self.display_scale,
        ) {
            (None, None, None, None, None, None, None) => None,
            (
                Some(x),
                Some(y),
                Some(display_x),
                Some(display_y),
                Some(display_width),
                Some(display_height),
                Some(display_scale),
            ) => {
                let placement = StoredPinPlacement {
                    x,
                    y,
                    display_name: self.display_name.clone(),
                    display_x,
                    display_y,
                    display_width,
                    display_height,
                    display_scale,
                };
                validate_placement(&placement).map_err(|error| error.to_string())?;
                Some(placement)
            }
            _ => return Err("位置字段不完整".to_string()),
        };
        let content = match content_type {
            ContentType::Image => StoredPinWorkspaceContent::Image {
                revision_id: self
                    .revision_id
                    .ok_or_else(|| "图片修订引用缺失".to_string())?,
                revision: super::StoredImageRevision {
                    source_png: self
                        .source_png
                        .clone()
                        .ok_or_else(|| "图片根图缺失".to_string())?,
                    source_width: self
                        .source_width
                        .ok_or_else(|| "图片宽度缺失".to_string())?,
                    source_height: self
                        .source_height
                        .ok_or_else(|| "图片高度缺失".to_string())?,
                    renderer_version: self
                        .renderer_version
                        .ok_or_else(|| "图片渲染器版本缺失".to_string())?,
                    annotations: serde_json::from_str(
                        self.annotations_json
                            .as_deref()
                            .ok_or_else(|| "图片标注文档缺失".to_string())?,
                    )
                    .map_err(|error| format!("图片标注文档损坏: {error}"))?,
                    adjustments: serde_json::from_str(
                        self.adjustments_json
                            .as_deref()
                            .ok_or_else(|| "图片调整文档缺失".to_string())?,
                    )
                    .map_err(|error| format!("图片调整文档损坏: {error}"))?,
                },
            },
            ContentType::Text | ContentType::Html => StoredPinWorkspaceContent::Snapshot {
                content_type,
                text_content: self.text_content.clone(),
                html_content: self.html_content.clone(),
                content_hash: self.content_hash.clone(),
            },
        };
        Ok(StoredPinWorkspaceItem {
            id: self.id,
            group_id: self.group_id,
            content,
            content_width: self.content_width,
            content_height: self.content_height,
            scale: self.scale,
            opacity: self.opacity,
            locked: self.locked == 1,
            above: self.above == 1,
            placement,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ImageRevisionWrite;

    fn register_revision(storage: &StorageEngine) -> i64 {
        storage
            .register_image_revision(ImageRevisionWrite {
                source_png: b"png-root",
                source_hash: &"1".repeat(64),
                source_width: 1,
                source_height: 1,
                renderer_version: 2,
                annotations_json: "[]",
                adjustments_json: "{}",
                document_hash: &"2".repeat(64),
                rendered_hash: &"3".repeat(64),
                root_clip_id: None,
            })
            .unwrap()
    }

    fn text_write<'a>(id: Option<i64>, group_id: Option<i64>) -> PinWorkspaceItemWrite<'a> {
        PinWorkspaceItemWrite {
            id,
            group_id,
            revision_id: None,
            content_type: ContentType::Text,
            text_content: Some("workspace text"),
            html_content: None,
            content_hash: "text-hash",
            content_width: 320.0,
            content_height: 180.0,
            scale: 1.25,
            opacity: 0.8,
            locked: true,
            above: false,
            placement: None,
        }
    }

    fn sample_placement() -> StoredPinPlacement {
        StoredPinPlacement {
            x: -1180.5,
            y: 42.25,
            display_name: Some("left".to_string()),
            display_x: -1920.0,
            display_y: 0.0,
            display_width: 1920.0,
            display_height: 1080.0,
            display_scale: 1.25,
        }
    }

    #[test]
    fn workspace_groups_delete_to_ungrouped_without_deleting_items() {
        let storage = StorageEngine::new_in_memory().unwrap();
        let group = storage.create_pin_group(" Research ").unwrap();
        assert_eq!(group.name, "Research");
        let item_id = storage
            .upsert_pin_workspace_item(text_write(None, Some(group.id)))
            .unwrap();
        assert!(storage.delete_pin_group(group.id).unwrap());
        let items = storage.load_pin_workspace_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, item_id);
        assert_eq!(items[0].group_id, None);
    }

    #[test]
    fn image_workspace_reuses_revision_and_round_trips_display_state() {
        let storage = StorageEngine::new_in_memory().unwrap();
        let revision_id = register_revision(&storage);
        let placement = sample_placement();
        let id = storage
            .upsert_pin_workspace_item(PinWorkspaceItemWrite {
                id: None,
                group_id: None,
                revision_id: Some(revision_id),
                content_type: ContentType::Image,
                text_content: None,
                html_content: None,
                content_hash: "",
                content_width: 640.0,
                content_height: 480.0,
                scale: 0.75,
                opacity: 0.55,
                locked: false,
                above: true,
                placement: Some(&placement),
            })
            .unwrap();
        let item = storage.load_pin_workspace_items().unwrap().remove(0);
        assert_eq!(item.id, id);
        assert_eq!(item.placement, Some(placement));
        let StoredPinWorkspaceContent::Image {
            revision_id: restored_id,
            revision,
        } = item.content
        else {
            panic!("应恢复图片修订");
        };
        assert_eq!(restored_id, revision_id);
        assert_eq!(revision.source_png, b"png-root");
    }

    #[test]
    fn corrupt_workspace_rows_are_skipped_without_hiding_valid_records() {
        let storage = StorageEngine::new_in_memory().unwrap();
        storage
            .upsert_pin_workspace_item(text_write(None, None))
            .unwrap();
        storage
            .conn
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        storage
            .conn
            .execute(
                "INSERT INTO pin_workspace_items(
                    content_type, content_hash, content_width, content_height,
                    scale, opacity, locked, above, created_at, updated_at)
                 VALUES ('text', 'broken', 10, 10, -9, 1, 0, 0, 0, 0)",
                [],
            )
            .unwrap();
        storage
            .conn
            .execute(
                "INSERT INTO pin_workspace_items(
                    content_type, content_hash, content_width, content_height,
                    scale, opacity, locked, above, created_at, updated_at)
                 VALUES ('text', 'wrong-sql-type', 'not-a-number', 10, 1, 1, 0, 0, 0, 0)",
                [],
            )
            .unwrap();
        let items = storage.load_pin_workspace_items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content_width, 320.0);
    }

    #[test]
    fn workspace_rejects_missing_groups_and_inconsistent_content_references() {
        let storage = StorageEngine::new_in_memory().unwrap();
        assert!(storage
            .upsert_pin_workspace_item(text_write(None, Some(999)))
            .is_err());
        let mut invalid = text_write(None, None);
        invalid.content_type = ContentType::Image;
        assert!(storage.upsert_pin_workspace_item(invalid).is_err());
        assert!(!storage.rename_pin_group(999, "missing").unwrap());
        assert!(!storage.delete_pin_workspace_item(999).unwrap());
    }

    #[test]
    fn presentation_updates_do_not_rewrite_workspace_content_identity() {
        let storage = StorageEngine::new_in_memory().unwrap();
        let id = storage
            .upsert_pin_workspace_item(PinWorkspaceItemWrite {
                id: None,
                group_id: None,
                revision_id: None,
                content_type: ContentType::Text,
                text_content: Some("stable"),
                html_content: None,
                content_hash: "stable-hash",
                content_width: 240.0,
                content_height: 120.0,
                scale: 1.0,
                opacity: 1.0,
                locked: false,
                above: false,
                placement: None,
            })
            .unwrap();
        let placement = sample_placement();
        assert!(storage
            .update_pin_workspace_presentation(
                id,
                &PinWorkspacePresentation {
                    scale: 1.75,
                    opacity: 0.55,
                    locked: true,
                    above: true,
                    placement: Some(placement.clone()),
                },
            )
            .unwrap());
        let item = storage.load_pin_workspace_items().unwrap().remove(0);
        assert_eq!(
            (item.scale, item.opacity, item.locked, item.above),
            (1.75, 0.55, true, true)
        );
        assert_eq!(item.placement, Some(placement));
        assert!(matches!(
            item.content,
            StoredPinWorkspaceContent::Snapshot { ref content_hash, .. } if content_hash == "stable-hash"
        ));

        assert!(storage
            .update_pin_workspace_presentation(
                id,
                &PinWorkspacePresentation {
                    scale: 1.5,
                    opacity: 0.8,
                    locked: false,
                    above: false,
                    placement: None,
                },
            )
            .unwrap());
        let item = storage.load_pin_workspace_items().unwrap().remove(0);
        assert_eq!(item.placement, Some(sample_placement()));
    }
}
