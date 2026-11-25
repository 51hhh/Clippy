use super::{editor, post, preview, App, Event};
use clippy_storage::BoundedImageData;
use image::RgbaImage;
use slint::{Image, Model};
use std::{collections::HashMap, thread};

const THUMBNAIL_EDGE: u32 = 64;
const MAX_VISIBLE_ROWS: usize = 32;

#[derive(Default)]
pub(super) struct Thumbnails {
    viewport: (usize, usize),
    images: HashMap<i64, Option<Image>>,
    in_flight: bool,
}

impl Thumbnails {
    pub(super) fn clear(&mut self) {
        self.viewport = (0, 0);
        self.images.clear();
    }
}

pub(super) enum Message {
    Viewport(i32, i32),
    Loaded(i64, Option<RgbaImage>),
}

fn loaded(id: i64, image: Option<RgbaImage>) {
    post(Event::Thumbnails(Message::Loaded(id, image)));
}

impl App {
    pub(super) fn thumbnails_message(&mut self, message: Message) {
        match message {
            Message::Viewport(first, end) => {
                let first = first as usize;
                self.thumbnails.viewport = (first, (end as usize).min(first + MAX_VISIBLE_ROWS));
            }
            Message::Loaded(id, image) => {
                self.thumbnails.in_flight = false;
                if self.window.is_none() {
                    return;
                }
                self.thumbnails
                    .images
                    .insert(id, image.as_ref().map(editor::to_slint));
            }
        }
        self.refresh_thumbnails();
    }

    pub(super) fn refresh_thumbnails(&mut self) {
        let Some(window) = self
            .window
            .as_ref()
            .filter(|window| !window.get_show_settings())
        else {
            return;
        };
        let rows = window.get_clips();
        let (first, end) = self.thumbnails.viewport;
        let mut visible = Vec::new();
        // 只保留当前视口的纹理，滚动几百项也不会累积整页图片。
        for (index, mut row) in rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.kind == "Image")
        {
            let id = row.id.parse().expect("历史列表持有数据库 ID");
            let wanted = !row.sensitive && (first..end).contains(&index);
            let mut thumbnail = Image::default();
            if wanted {
                visible.push(id);
                if let Some(Some(image)) = self.thumbnails.images.get(&id) {
                    thumbnail = image.clone();
                }
            }
            if row.thumbnail != thumbnail {
                row.thumbnail = thumbnail;
                rows.set_row_data(index, row);
            }
        }
        self.thumbnails.images.retain(|id, _| visible.contains(id));
        if self.thumbnails.in_flight || window.get_loading_history() {
            return;
        }
        let Some(id) = visible
            .into_iter()
            .find(|id| !self.thumbnails.images.contains_key(id))
        else {
            return;
        };
        self.thumbnails.in_flight = true;
        let result = self.storage.client().execute(
            move |db| db.get_bounded_image_snapshot(id, preview::MAX_PNG_BYTES),
            move |snapshot| {
                let Ok(Some(snapshot)) = snapshot else {
                    return loaded(id, None);
                };
                if snapshot.is_sensitive {
                    return loaded(id, None);
                }
                let BoundedImageData::Bytes(bytes) = snapshot.image else {
                    return loaded(id, None);
                };
                if thread::Builder::new()
                    .name("clippy-history-thumbnail".into())
                    .spawn(move || {
                        loaded(id, preview::thumbnail(&bytes, THUMBNAIL_EDGE).ok());
                    })
                    .is_err()
                {
                    loaded(id, None);
                }
            },
        );
        if result.is_err() {
            loaded(id, None);
        }
    }
}
