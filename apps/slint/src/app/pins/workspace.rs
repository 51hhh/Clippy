use super::{parse_id, send, Library, Message};
use crate::{ClippyTheme, WorkspaceGroupRow, WorkspaceRow, WorkspaceWindow};
use anyhow::Result;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::collections::HashMap;

const UNGROUPED: &str = "0";

pub(super) struct Workspace {
    pub(super) window: WorkspaceWindow,
    rows: Vec<WorkspaceRow>,
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
        window.on_move_to_group(|id, group| {
            let group = (group != UNGROUPED).then(|| parse_id(&group));
            send(Message::Move(parse_id(&id), group));
        });
        super::settings::connect_titlebar(&window, WorkspaceWindow::invoke_titlebar_contains);
        window.window().on_close_requested(|| {
            send(Message::CloseWorkspace);
            CloseRequestResponse::KeepWindowShown
        });
        window.show()?;
        Ok(Self {
            window,
            rows: Vec::new(),
        })
    }

    pub(super) fn update(&mut self, (groups, summaries): Library) {
        let names: HashMap<_, _> = groups
            .iter()
            .map(|group| (group.id, group.name.as_str()))
            .collect();
        let mut counts = HashMap::<Option<i64>, i32>::new();
        self.rows = summaries
            .into_iter()
            .map(|item| {
                let group = item.group_id.filter(|id| names.contains_key(id));
                *counts.entry(group).or_default() += 1;
                WorkspaceRow {
                    id: item.id.to_string().into(),
                    label: item
                        .preview_text
                        .unwrap_or_else(|| {
                            format!(
                                "Image {} × {}",
                                item.content_width as u32, item.content_height as u32,
                            )
                        })
                        .into(),
                    group: group.map_or("Ungrouped", |id| names[&id]).into(),
                    group_key: group.map_or_else(|| UNGROUPED.into(), |id| id.to_string().into()),
                }
            })
            .collect();

        let mut group_rows = vec![
            WorkspaceGroupRow {
                key: "".into(),
                label: "All pins".into(),
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
        self.filter(filter);
    }

    pub(super) fn filter(&self, group: SharedString) {
        let visible: Vec<_> = self
            .rows
            .iter()
            .filter(|row| group.is_empty() || row.group_key == group)
            .cloned()
            .collect();
        if !visible
            .iter()
            .any(|row| row.id == self.window.get_selection())
        {
            self.window.set_selection(
                visible
                    .first()
                    .map(|row| row.id.clone())
                    .unwrap_or_default(),
            );
        }
        self.window.set_group_filter(group);
        self.window.set_pins(ModelRc::new(VecModel::from(visible)));
    }
}
