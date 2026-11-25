use super::{decode_revision, editor, parse_id, send, Library, Message, View};
use crate::{ClippyTheme, WorkspaceGroupRow, WorkspaceRow, WorkspaceWindow};
use anyhow::Result;
use clippy_core::models::ContentType;
use clippy_storage::{StorageClient, StoredPinWorkspaceContent};
use image::{DynamicImage, RgbaImage};
use slint::{CloseRequestResponse, ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::{collections::HashMap, ops::Range, rc::Rc, sync::Arc, thread};

const UNGROUPED: &str = "0";
const THUMBNAIL_EDGE: u32 = 288;
const MAX_THUMBNAILS: usize = 16;
const MAX_THUMBNAIL_SOURCE_PIXELS: u32 = 48 * 1024 * 1024 / 4;

pub(super) struct Workspace {
    pub(super) window: WorkspaceWindow,
    rows: Vec<WorkspaceRow>,
    filtered: Rc<VecModel<WorkspaceRow>>,
    visible: Range<usize>,
    thumbnails: HashMap<String, (usize, Option<Image>)>,
}

impl Workspace {
    pub(super) fn open(theme: i32) -> Result<Self> {
        let window = WorkspaceWindow::new()?;
        window.global::<ClippyTheme>().set_index(theme);
        window.on_close_workspace(|| send(Message::CloseWorkspace));
        window.on_open_pin(|id| send(Message::Open(parse_id(&id))));
        window.on_delete_pin(|id| send(Message::Remove(parse_id(&id))));
        window.on_select_group(|group| send(Message::Filter(group.to_string())));
        window.on_create_group(|name| send(Message::CreateGroup(name.to_string())));
        window.on_rename_group(|id, name| {
            send(Message::RenameGroup(parse_id(&id), name.to_string()))
        });
        window.on_delete_group(|id| send(Message::DeleteGroup(parse_id(&id))));
        window.on_viewport(|first, end| {
            send(Message::Viewport(
                first.max(0) as usize..end.max(0) as usize,
            ))
        });
        window.on_move_to_group(|id, group| {
            let group = (group != UNGROUPED).then(|| parse_id(&group));
            send(Message::Move(parse_id(&id), group));
        });
        super::settings::connect_titlebar(&window, WorkspaceWindow::invoke_titlebar_contains);
        window.window().on_close_requested(|| {
            send(Message::CloseWorkspace);
            CloseRequestResponse::KeepWindowShown
        });
        let filtered = Rc::new(VecModel::default());
        window.set_pins(ModelRc::from(filtered.clone()));
        window.show()?;
        Ok(Self {
            window,
            rows: Vec::new(),
            filtered,
            visible: 0..4,
            thumbnails: HashMap::new(),
        })
    }

    pub(super) fn update(&mut self, (groups, summaries): Library, views: &HashMap<i64, View>) {
        let indices: HashMap<_, _> = groups
            .iter()
            .enumerate()
            .map(|(index, group)| (group.id, index as i32 + 1))
            .collect();
        let mut counts = HashMap::<Option<i64>, i32>::new();
        self.rows = summaries
            .into_iter()
            .map(|item| {
                let group = item.group_id.filter(|id| indices.contains_key(id));
                *counts.entry(group).or_default() += 1;
                WorkspaceRow {
                    id: item.id.to_string().into(),
                    label: item.preview_text.unwrap_or_default().into(),
                    detail: format!(
                        "{} × {}",
                        item.content_width as u32, item.content_height as u32
                    )
                    .into(),
                    group_key: group.map_or_else(|| UNGROUPED.into(), |id| id.to_string().into()),
                    group_index: group.map_or(0, |id| indices[&id]),
                    is_image: item.content_type == ContentType::Image,
                    thumbnail: Image::default(),
                    opened: views.contains_key(&item.id),
                }
            })
            .collect();

        let mut group_rows = vec![
            WorkspaceGroupRow {
                key: "".into(),
                label: "All".into(),
                count: self.rows.len() as i32,
            },
            WorkspaceGroupRow {
                key: UNGROUPED.into(),
                label: "Ungrouped".into(),
                count: counts.get(&None).copied().unwrap_or(0),
            },
        ];
        group_rows.extend(groups.into_iter().map(|group| WorkspaceGroupRow {
            key: group.id.to_string().into(),
            label: group.name.into(),
            count: counts.get(&Some(group.id)).copied().unwrap_or(0),
        }));
        let current = self.window.get_group_filter();
        let filter = if group_rows.iter().any(|group| group.key == current) {
            current
        } else {
            SharedString::default()
        };
        let destinations = group_rows
            .iter()
            .skip(1)
            .map(|group| group.label.clone())
            .collect::<Vec<_>>();
        self.window
            .set_groups(ModelRc::new(VecModel::from(group_rows)));
        self.window
            .set_destinations(ModelRc::new(VecModel::from(destinations)));
        self.window.set_status(SharedString::default());
        self.filter(filter);
    }

    pub(super) fn filter(&mut self, group: SharedString) {
        let name = if group.is_empty() || group == UNGROUPED {
            SharedString::default()
        } else {
            self.window
                .get_groups()
                .iter()
                .find(|row| row.key == group)
                .map_or_else(SharedString::default, |row| row.label)
        };
        if self.window.get_group_filter() != group {
            self.thumbnails.clear();
            self.visible = 0..self.visible.len();
        }
        let rows: Vec<_> = self
            .rows
            .iter()
            .filter(|row| group.is_empty() || row.group_key == group)
            .cloned()
            .enumerate()
            .map(|(index, mut row)| {
                if let Some((position, image)) = self.thumbnails.get_mut(row.id.as_str()) {
                    *position = index;
                    row.thumbnail = image.clone().unwrap_or_default();
                }
                row
            })
            .collect();
        self.filtered.set_vec(rows);
        self.window.set_group_name(name);
        self.window.set_group_filter(group);
    }

    pub(super) fn viewport(&mut self, range: Range<usize>) {
        self.visible = range;
    }

    fn visible_rows(&self) -> impl Iterator<Item = (usize, WorkspaceRow)> + '_ {
        self.visible
            .clone()
            .take(MAX_THUMBNAILS)
            .map_while(|index| self.filtered.row_data(index).map(|row| (index, row)))
    }

    pub(super) fn thumbnail_ready(&mut self, id: i64, image: Option<RgbaImage>) {
        let id = id.to_string();
        let Some((index, mut row)) = self.visible_rows().find(|(_, row)| row.id == id) else {
            return;
        };
        let image = image.map(|rgba| editor::to_slint(&rgba));
        if let Some(image) = image.as_ref() {
            row.thumbnail = image.clone();
            self.filtered.set_row_data(index, row);
        }
        self.thumbnails.insert(id, (index, image));
    }

    pub(super) fn request_thumbnail(&mut self, storage: StorageClient) -> Result<bool> {
        let rows: Vec<_> = self.visible_rows().collect();
        self.thumbnails.retain(|id, (index, image)| {
            if rows.iter().any(|(_, row)| row.id == *id) {
                return true;
            }
            if image.is_some() {
                if let Some(mut row) = self.filtered.row_data(*index).filter(|row| row.id == *id) {
                    row.thumbnail = Image::default();
                    self.filtered.set_row_data(*index, row);
                }
            }
            false
        });
        let Some((_, row)) = rows
            .into_iter()
            .find(|(_, row)| row.is_image && !self.thumbnails.contains_key(row.id.as_str()))
        else {
            return Ok(false);
        };
        let id = parse_id(&row.id);
        storage.execute(
            move |db| {
                db.load_pin_workspace_preview(
                    id,
                    super::super::preview::MAX_PNG_BYTES as u32,
                    MAX_THUMBNAIL_SOURCE_PIXELS,
                )
            },
            move |result| {
                if let Err(error) = thread::Builder::new()
                    .name("clippy-pin-thumbnail".into())
                    .spawn(move || {
                        let result = result.map_err(anyhow::Error::from).and_then(|item| {
                            let Some(StoredPinWorkspaceContent::Image { revision, .. }) =
                                item.map(|item| item.content)
                            else {
                                return Ok(None);
                            };
                            let (image, _) = decode_revision(revision)?;
                            let thumbnail = DynamicImage::ImageRgba8(Arc::unwrap_or_clone(image))
                                .thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
                                .into_rgba8();
                            Ok(Some(thumbnail))
                        });
                        send(Message::Thumbnail(id, result));
                    })
                {
                    send(Message::Thumbnail(id, Err(error.into())));
                }
            },
        )?;
        Ok(true)
    }
}
