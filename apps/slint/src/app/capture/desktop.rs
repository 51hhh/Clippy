use super::{post, Area, Event, View};
use crate::{CaptureWindow, DesktopPreview};
use clippy_capture::render_v2::MAX_RENDERER_PIXELS;
use clippy_platform::capture::desktop::{Composition, DesktopSelection, OutputGeometry};
use slint::{ComponentHandle, Weak};
use std::{cell::Cell, rc::Rc};

struct Screen {
    window: Weak<CaptureWindow>,
    geometry: OutputGeometry,
}

struct DesktopDrag {
    screens: Vec<Screen>,
    showing: Cell<bool>,
    composition: Cell<Option<Composition>>,
}

pub(super) fn connect(views: &[View]) {
    let drag = Rc::new(DesktopDrag {
        screens: views
            .iter()
            .map(|view| Screen {
                window: view.window.as_weak(),
                geometry: view.frame.output_geometry(),
            })
            .collect(),
        showing: Cell::new(false),
        composition: Cell::new(None),
    });
    for (index, view) in views.iter().enumerate() {
        let window = &view.window;
        window.set_desktop_enabled(true);
        window.on_begin_desktop_selection({
            let drag = drag.clone();
            move || drag.begin(index)
        });
        window.on_update_desktop_selection({
            let drag = drag.clone();
            move |x0, y0, x1, y1, released| drag.update(index, [x0, y0], [x1, y1], released)
        });
        window.on_reset_desktop_selection({
            let drag = drag.clone();
            move || drag.clear()
        });
        window.on_confirm_desktop({
            let drag = drag.clone();
            move |action| {
                let composition = drag
                    .composition
                    .get()
                    .expect("跨屏工具栏只提交已校验的画布");
                post(Event::CaptureSelection {
                    area: Area::Desktop(composition),
                    action,
                });
            }
        });
    }
}

impl DesktopDrag {
    fn begin(&self, index: usize) {
        self.clear();
        for (i, screen) in self.screens.iter().enumerate() {
            screen.window.unwrap().set_owns_selection(i == index);
        }
    }

    fn clear(&self) {
        if self.showing.replace(false) {
            self.composition.set(None);
            for screen in &self.screens {
                screen
                    .window
                    .unwrap()
                    .set_desktop_preview(DesktopPreview::default());
            }
        }
    }

    fn update(&self, index: usize, start: [f32; 2], end: [f32; 2], released: bool) {
        let origin = self.screens[index].geometry;
        let end = origin.desktop_point(end);
        let area = DesktopSelection::between(origin.desktop_point(start), end);
        let crosses = self.screens.iter().any(|screen| {
            area.intersect(screen.geometry.desktop)
                .is_some_and(|part| part.intersect(origin.desktop) != Some(part))
        });
        if !crosses {
            self.clear();
            return;
        }
        self.showing.set(true);
        let composition = if released {
            area.composition(
                self.screens.iter().map(|screen| screen.geometry),
                MAX_RENDERER_PIXELS,
            )
            .ok()
        } else {
            None
        };
        self.composition.set(composition);
        let size = composition.map_or((0, 0), Composition::size);
        let controls = released.then(|| {
            self.screens
                .iter()
                .position(|screen| {
                    let rect = screen.geometry.desktop;
                    (f64::from(rect.x)..f64::from(rect.x) + f64::from(rect.width)).contains(&end[0])
                        && (f64::from(rect.y)..f64::from(rect.y) + f64::from(rect.height))
                            .contains(&end[1])
                })
                .unwrap_or(index)
        });
        for (i, screen) in self.screens.iter().enumerate() {
            let rect = screen.geometry.desktop;
            let mut preview = DesktopPreview {
                active: true,
                dragging: !released,
                controls: controls == Some(i),
                pixel_width: size.0 as i32,
                pixel_height: size.1 as i32,
                ..Default::default()
            };
            if let Some(part) = area.intersect(rect) {
                [preview.left, preview.top, preview.right, preview.bottom] =
                    part.relative_bounds(rect);
                preview.edge_left = part.left == area.left;
                preview.edge_top = part.top == area.top;
                preview.edge_right = part.right == area.right;
                preview.edge_bottom = part.bottom == area.bottom;
            }
            screen.window.unwrap().set_desktop_preview(preview);
        }
    }
}
