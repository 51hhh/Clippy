use super::{editor, post, Event};
use crate::{
    CaptureAnnotation, CaptureMark, CaptureMosaic, CapturePoint, CaptureTool, CaptureWindow,
    CaptureWindowTarget, ClippyTheme,
};
use anyhow::{Context, Result};
use clippy_capture::{
    editor::{Editor, ImageProject, Tool},
    project::MAX_TEXT_BYTES,
    render_v2::{mosaic_preview, DEFAULT_MOSAIC_CELL, MAX_EFFECT_WORK_PIXELS, MAX_RENDERER_PIXELS},
};
use clippy_core::image_output::{filename_candidates, SaveTarget};
use clippy_platform::capture::{
    self,
    desktop::{Composition, Frame, OutputGeometry},
    window_pick, CaptureSource, DesktopRect, FrozenCapture, FrozenOutput, Region,
    RegionCancellation,
};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageBuffer, ImageEncoder, RgbaImage,
};
use slint::winit_030::{
    winit::{
        event::WindowEvent,
        event_loop::ActiveEventLoop,
        window::{Fullscreen, WindowId, WindowLevel},
    },
    EventResult, WinitWindowAccessor,
};
use slint::{
    CloseRequestResponse, ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, VecModel,
};
#[cfg(target_os = "macos")]
use slint::{LogicalPosition, LogicalSize};
#[cfg(not(target_os = "macos"))]
use slint::{PhysicalPosition, PhysicalSize};
use std::{
    cell::RefCell,
    fmt::Write as _,
    fs, io,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

mod desktop;
mod scan;

pub(super) use scan::Message as ScanMessage;

pub(super) struct Selection {
    views: Vec<View>,
    source: CaptureSource,
    started: Instant,
    scan: Rc<scan::Overlay>,
}

impl Selection {
    pub(super) fn install_window_targets(
        &self,
        started: Instant,
        result: Result<Vec<Vec<window_pick::WindowTarget>>>,
        diagnostics: bool,
    ) {
        if self.started != started {
            return;
        }
        let targets = match result {
            Ok(targets) => targets,
            Err(error) => {
                if diagnostics {
                    eprintln!("capture_windows unavailable: {error}");
                }
                return;
            }
        };
        if diagnostics {
            eprintln!(
                "capture_windows candidates={} elapsed_ms={:.1}",
                targets.iter().map(Vec::len).sum::<usize>(),
                started.elapsed().as_secs_f64() * 1000.0,
            );
        }
        for (view, targets) in self.views.iter().zip(targets) {
            let targets: Vec<_> = targets
                .into_iter()
                .map(|target| {
                    let [left, top, right, bottom] = target.pixels.map(|edge| edge as i32);
                    CaptureWindowTarget {
                        left,
                        top,
                        right,
                        bottom,
                        title: target.title.into(),
                    }
                })
                .collect();
            view.window
                .set_window_targets(ModelRc::new(VecModel::from(targets)));
        }
    }
}

struct View {
    window: CaptureWindow,
    frame: OverlayFrame,
}

struct OverlayFrame {
    pixels: SharedPixelBuffer<Rgba8Pixel>,
    name: String,
    geometry: DesktopRect,
}

impl OverlayFrame {
    fn output_geometry(&self) -> OutputGeometry {
        OutputGeometry {
            desktop: self.geometry,
            pixels: (self.pixels.width(), self.pixels.height()),
        }
    }
}

impl From<FrozenOutput> for OverlayFrame {
    fn from(output: FrozenOutput) -> Self {
        let pixels = &output.pixels;
        // 转成 Slint 缓冲后释放采集帧，预览和裁剪共同持有同一份像素。
        Self {
            pixels: SharedPixelBuffer::clone_from_slice(&pixels.rgba, pixels.width, pixels.height),
            name: output.name,
            geometry: output.geometry,
        }
    }
}

pub(super) enum StillDestination {
    Clipboard,
    File(SaveTarget),
    Editor,
}

pub(super) enum Destination {
    Still(StillDestination),
    Longshot,
    Record,
}

pub(super) enum Area {
    Display { index: usize, bounds: [f32; 4] },
    Desktop(Composition),
}

pub(super) type EditedCapture = (Arc<RgbaImage>, ImageProject);

pub(super) struct PendingOutput {
    pub window_id: WindowId,
    started: Instant,
    clearing_overlay: Option<View>,
    job: OutputJob,
}

impl PendingOutput {
    pub fn is_clearing(&self, started: Instant) -> bool {
        self.started == started && self.clearing_overlay.is_some()
    }

    pub fn overlay_cleared(&mut self, started: Instant) {
        if self.is_clearing(started) {
            self.clearing_overlay.take();
        }
    }

    pub fn start(mut self, closed: Instant) -> Result<()> {
        anyhow::ensure!(
            self.clearing_overlay.is_none(),
            "The capture overlay closed before its transparent frame"
        );
        self.job.region.source.overlay_closed(closed)?;
        self.job.start()?;
        Ok(())
    }
}

struct OutputJob {
    region: Region,
    marks: Vec<OutputMark>,
    pixels: SharedPixelBuffer<Rgba8Pixel>,
    destination: Destination,
}

struct OutputMark {
    geometry: CaptureMark,
    points: Vec<[f32; 2]>,
    text: String,
}

impl From<CaptureAnnotation> for OutputMark {
    fn from(mark: CaptureAnnotation) -> Self {
        Self {
            geometry: mark.geometry,
            points: mark.points.iter().map(|point| [point.x, point.y]).collect(),
            text: mark.text.to_string(),
        }
    }
}

#[derive(Default)]
struct PenStroke {
    points: Vec<CapturePoint>,
    path: String,
}

impl PenStroke {
    fn sample(&mut self, point: CapturePoint, width: f32, height: f32) -> bool {
        if self.points.len() == editor::MAX_STROKE_POINTS || self.points.last() == Some(&point) {
            return false;
        }
        let command = if self.points.is_empty() { 'M' } else { 'L' };
        write!(
            self.path,
            "{command}{:.2} {:.2} ",
            point.x * width,
            point.y * height
        )
        .expect("写入 String 无 I/O");
        self.points.push(point);
        true
    }
}

pub(super) struct CaptureWindowEvents;

impl slint::winit_030::CustomApplicationHandler for CaptureWindowEvents {
    fn window_event(
        &mut self,
        _: &ActiveEventLoop,
        id: WindowId,
        _: Option<&slint::winit_030::winit::window::Window>,
        _: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        if matches!(event, WindowEvent::Destroyed) {
            post(Event::CaptureWindowDestroyed(id, Instant::now()));
        }
        EventResult::Propagate
    }
}

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

pub(super) fn start(started: Instant, cancellation: RegionCancellation) -> io::Result<()> {
    thread::Builder::new()
        .name("clippy-freeze".into())
        .spawn(move || {
            capture::freeze_selection(&cancellation, |result| {
                let outputs = result
                    .as_ref()
                    .ok()
                    .filter(|capture| !capture.source.uses_portal())
                    .map(|capture| {
                        capture
                            .outputs
                            .iter()
                            .map(|output| {
                                (output.geometry, (output.pixels.width, output.pixels.height))
                            })
                            .collect::<Vec<_>>()
                    });
                post(Event::CaptureReady { started, result });
                if let Some(outputs) = outputs {
                    post(Event::CaptureWindows {
                        started,
                        result: window_pick::for_outputs(&outputs),
                    });
                }
            });
        })?;
    Ok(())
}

pub(super) fn present(
    capture: FrozenCapture,
    started: Instant,
    diagnostics: bool,
    theme: i32,
) -> Result<Selection> {
    let single_output = capture.output_count == 1;
    let portal_stream = capture.source.uses_portal();
    let views: Vec<_> = capture
        .outputs
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let wayland = clippy_platform::session::is_wayland();
            let window = CaptureWindow::new()?;
            window.global::<ClippyTheme>().set_index(theme);
            window.set_portal_stream(portal_stream);
            let frame = OverlayFrame::from(output);
            let view = View { window, frame };
            let geometry = view.frame.geometry;
            let pixels = &view.frame.pixels;
            let source_size = (pixels.width(), pixels.height());
            view.window.set_source_width(pixels.width() as i32);
            view.window.set_source_height(pixels.height() as i32);
            view.window
                .set_frame(slint::Image::from_rgba8(pixels.clone()));
            #[cfg(target_os = "macos")]
            {
                // Core Graphics 的全局坐标是逻辑点，Retina 帧保留独立的源像素尺寸。
                let window = view.window.window();
                window.set_position(LogicalPosition::new(geometry.x as f32, geometry.y as f32));
                window.set_size(LogicalSize::new(
                    geometry.width as f32,
                    geometry.height as f32,
                ));
            }
            #[cfg(not(target_os = "macos"))]
            {
                let window = view.window.window();
                window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
                let (width, height) = if wayland {
                    source_size
                } else {
                    (geometry.width, geometry.height)
                };
                window.set_size(PhysicalSize::new(width, height));
            }
            view.window.on_cancel(|| post(Event::CaptureCancel));
            view.window.on_window_at({
                let window = view.window.as_weak();
                move |x, y| {
                    window
                        .unwrap()
                        .get_window_targets()
                        .iter()
                        .position(|target| {
                            x >= target.left as f32
                                && x < target.right as f32
                                && y >= target.top as f32
                                && y < target.bottom as f32
                        })
                        .map_or(-1, |index| index as i32)
                }
            });
            configure_annotations(&view.window, pixels);
            view.window.window().on_close_requested(|| {
                post(Event::CaptureCancel);
                CloseRequestResponse::HideWindow
            });
            view.window.on_confirm({
                let window = view.window.as_weak();
                move |action, left, top, right, bottom| {
                    post(Event::CaptureSelection {
                        action,
                        area: Area::Display {
                            index,
                            bounds: capture_bounds(
                                &window.unwrap(),
                                source_size,
                                [left, top, right, bottom],
                            ),
                        },
                    });
                }
            });
            let ready_name = view.frame.name.clone();
            let placement_window = view.window.as_weak();
            let mut needs_placement = wayland;
            let mut first_redraw = true;
            let mut blank_frame_submitted = false;
            view.window
                .window()
                .on_winit_window_event(move |window, event| {
                    if needs_placement {
                        // Wayland 的原生窗口由事件循环建立，show 返回时可能还没有。
                        if let Some(result) = window.with_winit_window(|native| {
                            let matching = native.available_monitors().find(|monitor| {
                                monitor.name().as_deref() == Some(ready_name.as_str())
                            });
                            if matching.is_none()
                                && (!single_output || native.available_monitors().count() > 1)
                            {
                                anyhow::bail!("Cannot match Wayland output {ready_name}");
                            }
                            native.set_fullscreen(Some(Fullscreen::Borderless(matching)));
                            anyhow::Ok(())
                        }) {
                            needs_placement = false;
                            if let Err(error) = result {
                                post(Event::CapturePlacementFailed(
                                    placement_window.clone(),
                                    error,
                                ));
                                return EventResult::PreventDefault;
                            }
                        }
                    }
                    if matches!(
                        event,
                        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. }
                    ) {
                        if let Some(overlay) = placement_window.upgrade() {
                            overlay.invoke_invalidate_scan();
                        }
                    }
                    if matches!(event, WindowEvent::RedrawRequested) {
                        if diagnostics && first_redraw {
                            first_redraw = false;
                            eprintln!(
                                "capture_overlay output={ready_name} elapsed_ms={:.1}",
                                started.elapsed().as_secs_f64() * 1000.0
                            );
                        }
                        if portal_stream
                            && placement_window
                                .upgrade()
                                .is_some_and(|overlay| overlay.get_blank())
                        {
                            if blank_frame_submitted {
                                post(Event::CaptureOverlayCleared(started));
                                return EventResult::PreventDefault;
                            }
                            // Wayland 的下一次重绘由本次提交的帧回调推进。
                            blank_frame_submitted = true;
                            window.with_winit_window(|native| native.request_redraw());
                        }
                    }
                    EventResult::Propagate
                });
            view.window.show()?;
            if !wayland {
                view.window.window().with_winit_window(|native| {
                    native.set_window_level(WindowLevel::AlwaysOnTop);
                });
            }
            Ok(view)
        })
        .collect::<Result<_>>()?;
    let scan = scan::connect(&views, started);
    if views.len() > 1 {
        desktop::connect(&views, &scan);
    }
    Ok(Selection {
        views,
        source: capture.source,
        started,
        scan,
    })
}

fn capture_bounds(window: &CaptureWindow, size: (u32, u32), manual: [f32; 4]) -> [f32; 4] {
    let index = window.get_snapped_window();
    if index < 0 {
        return manual;
    }
    let target = window
        .get_window_targets()
        .row_data(index as usize)
        .expect("吸附目标来自当前覆盖层的窗口列表");
    window_pick::normalized_pixel_bounds(
        [target.left, target.top, target.right, target.bottom].map(|edge| edge as u32),
        size,
    )
}

fn configure_annotations(window: &CaptureWindow, source: &SharedPixelBuffer<Rgba8Pixel>) {
    let marks = Rc::new(VecModel::default());
    let undone = Rc::new(VecModel::default());
    window.set_marks(ModelRc::from(marks.clone()));
    window.set_undone_marks(ModelRc::from(undone.clone()));
    window.on_add_mark({
        let marks = marks.clone();
        let undone = undone.clone();
        let window = window.as_weak();
        move |mark| {
            let window = window.unwrap();
            if mark.geometry.tool == CaptureTool::Mosaic {
                let work: u64 = marks
                    .iter()
                    .map(|entry| entry.mosaic.work_pixels as u64)
                    .sum();
                if work + mark.mosaic.work_pixels as u64 > MAX_EFFECT_WORK_PIXELS {
                    window
                        .set_annotation_error("Remove a mosaic area before adding another".into());
                    return false;
                }
            }
            if mark.geometry.tool == CaptureTool::Text {
                if mark.text.len() > MAX_TEXT_BYTES {
                    window.set_annotation_error("Enter a shorter annotation".into());
                    return false;
                }
                if mark.text.trim().is_empty() {
                    return true;
                }
            }
            window.set_annotation_error(Default::default());
            undone.set_vec(Vec::new());
            marks.push(mark);
            window.invoke_invalidate_scan();
            true
        }
    });
    window.on_undo_mark({
        let marks = marks.clone();
        let undone = undone.clone();
        let window = window.as_weak();
        move || {
            undone.push(take_last_mark(&marks));
            window.unwrap().invoke_invalidate_scan();
        }
    });
    window.on_redo_mark({
        let marks = marks.clone();
        let undone = undone.clone();
        let window = window.as_weak();
        move || {
            marks.push(take_last_mark(&undone));
            window.unwrap().invoke_invalidate_scan();
        }
    });
    window.on_clear_marks({
        let window = window.as_weak();
        move || {
            marks.set_vec(Vec::new());
            undone.set_vec(Vec::new());
            window.unwrap().invoke_invalidate_scan();
        }
    });
    let pen = Rc::new(RefCell::new(PenStroke::default()));
    window.on_sample_pen({
        let pen = pen.clone();
        let window = window.as_weak();
        move |begin, x, y, width, height| {
            let mut pen = pen.borrow_mut();
            if begin {
                pen.points.clear();
                pen.path.clear();
            }
            if pen.sample(CapturePoint { x, y }, width, height) {
                window.unwrap().set_pen_path(pen.path.as_str().into());
            }
        }
    });
    window.on_finish_pen({
        let window = window.as_weak();
        move |geometry| {
            let window = window.unwrap();
            let PenStroke { points, path } = std::mem::take(&mut *pen.borrow_mut());
            window.set_pen_path(Default::default());
            if points.len() > 1 {
                window.invoke_add_mark(CaptureAnnotation {
                    geometry,
                    points: ModelRc::new(VecModel::from(points)),
                    path: path.into(),
                    ..Default::default()
                });
            }
        }
    });
    window.on_render_mosaic({
        let source = source.clone();
        let window = window.as_weak();
        move |mark, left, top, right, bottom| {
            let window = window.unwrap();
            let bounds = capture_bounds(
                &window,
                (source.width(), source.height()),
                [left, top, right, bottom],
            );
            if let Err(error) = start_mosaic(&window, &source, mark, bounds) {
                window.set_annotation_error(format!("Could not apply mosaic: {error}").into());
            }
        }
    });
}

fn start_mosaic(
    window: &CaptureWindow,
    source: &SharedPixelBuffer<Rgba8Pixel>,
    geometry: CaptureMark,
    selection: [f32; 4],
) -> Result<()> {
    let viewport = capture::pixel_rect(source.width(), source.height(), selection)?;
    let width = viewport[2] - viewport[0];
    let height = viewport[3] - viewport[1];
    anyhow::ensure!(
        u64::from(width) * u64::from(height) <= MAX_RENDERER_PIXELS,
        "Select a smaller area"
    );
    let source = source.clone();
    let handle = window.as_weak();
    thread::Builder::new()
        .name("clippy-mosaic-preview".into())
        .spawn(move || {
            let pixels = ImageBuffer::from_raw(source.width(), source.height(), source.as_bytes())
                .expect("冻结帧已校验尺寸与 RGBA 布局");
            let preview = mosaic_preview(
                &pixels,
                viewport,
                [geometry.x0, geometry.y0],
                [geometry.x1, geometry.y1],
            );
            let [x0, y0, x1, y1] = preview.bounds;
            // 关闭覆盖层会取消交付，工作线程只持有源缓冲直到本次采样结束。
            let _ = handle.upgrade_in_event_loop(move |window| {
                let mosaic = CaptureMosaic {
                    source: editor::to_slint(&preview.tiles),
                    x: x0 as f32 / width as f32,
                    y: y0 as f32 / height as f32,
                    width: (x1 - x0) as f32 / width as f32,
                    height: (y1 - y0) as f32 / height as f32,
                    tiled_width: (preview.tiles.width() * DEFAULT_MOSAIC_CELL) as f32
                        / width as f32,
                    tiled_height: (preview.tiles.height() * DEFAULT_MOSAIC_CELL) as f32
                        / height as f32,
                    work_pixels: ((x1 - x0) * (y1 - y0)) as i32,
                };
                window.invoke_add_mark(CaptureAnnotation {
                    geometry,
                    mosaic,
                    ..Default::default()
                });
                window.set_mosaic_pending(false);
            });
        })?;
    window.set_annotation_error(Default::default());
    window.set_mosaic_pending(true);
    Ok(())
}

pub(super) fn output(
    selection: Selection,
    area: Area,
    destination: Destination,
) -> io::Result<Option<PendingOutput>> {
    let (index, bounds) = match area {
        Area::Display { index, bounds } => (index, bounds),
        Area::Desktop(composition) => {
            let Destination::Still(destination) = destination else {
                unreachable!("跨屏工具栏只提供静态截图动作");
            };
            output_desktop(selection, composition, destination)?;
            return Ok(None);
        }
    };
    let Selection {
        mut views,
        source,
        started,
        ..
    } = selection;
    let selected = views.swap_remove(index);
    let needs_stream = matches!(destination, Destination::Longshot | Destination::Record);
    let window_id = if needs_stream && source.uses_portal() {
        Some(
            selected
                .window
                .window()
                .with_winit_window(|window| window.id())
                .expect("截图确认事件来自已映射的窗口"),
        )
    } else {
        None
    };
    let region = Region {
        output: selected.frame.name.clone(),
        geometry: selected.frame.geometry,
        source_size: (
            selected.frame.pixels.width(),
            selected.frame.pixels.height(),
        ),
        bounds,
        source: if needs_stream {
            source
        } else {
            let _closed_source = source;
            CaptureSource::default()
        },
    };
    let marks = if needs_stream {
        Vec::new()
    } else {
        selected
            .window
            .get_marks()
            .iter()
            .map(OutputMark::from)
            .collect()
    };
    let pixels = selected.frame.pixels.clone();
    views.clear();
    let job = OutputJob {
        region,
        marks,
        pixels,
        destination,
    };
    if let Some(window_id) = window_id {
        // 先提交透明画面，避免合成器用带选框的旧画面播放窗口关闭动画。
        selected.window.set_blank(true);
        selected
            .window
            .window()
            .with_winit_window(|native| native.request_redraw());
        slint::Timer::single_shot(Duration::from_secs(1), move || {
            post(Event::CaptureOverlayClearTimedOut(started));
        });
        return Ok(Some(PendingOutput {
            window_id,
            started,
            clearing_overlay: Some(selected),
            job,
        }));
    }
    drop(selected);
    job.start()?;
    Ok(None)
}

fn output_desktop(
    selection: Selection,
    composition: Composition,
    destination: StillDestination,
) -> io::Result<()> {
    let snapshot = DesktopSnapshot::new(&selection.views, composition);
    drop(selection);
    thread::Builder::new()
        .name("clippy-screenshot-output".into())
        .spawn(move || {
            let started = Instant::now();
            finish_still(Ok(snapshot.render()), &[], destination, started);
        })?;
    Ok(())
}

struct DesktopSnapshot {
    composition: Composition,
    frames: Vec<(OutputGeometry, SharedPixelBuffer<Rgba8Pixel>)>,
}

impl DesktopSnapshot {
    fn new(views: &[View], composition: Composition) -> Self {
        let frames = views
            .iter()
            .filter(|view| composition.intersects(view.frame.geometry))
            .map(|view| (view.frame.output_geometry(), view.frame.pixels.clone()))
            .collect();
        Self {
            composition,
            frames,
        }
    }

    fn render(self) -> RgbaImage {
        self.composition
            .render(self.frames.iter().map(|(output, pixels)| Frame {
                output: *output,
                rgba: pixels.as_bytes(),
            }))
    }
}

impl OutputJob {
    fn start(self) -> io::Result<()> {
        let Self {
            region,
            marks,
            pixels,
            destination,
        } = self;
        let bounds = region.bounds;
        thread::Builder::new()
            .name("clippy-screenshot-output".into())
            .spawn(move || {
                let started = Instant::now();
                let (width, height) = (pixels.width(), pixels.height());
                if matches!(destination, Destination::Record) {
                    post(Event::RecordingStart(
                        capture::pixel_rect(width, height, bounds).map(
                            |[left, top, right, bottom]| (region, (right - left, bottom - top)),
                        ),
                    ));
                    return;
                }
                let result = capture::crop_rgba(pixels.as_bytes(), width, height, bounds);
                match destination {
                    Destination::Still(destination) => {
                        finish_still(result, &marks, destination, started)
                    }
                    Destination::Longshot => {
                        post(Event::LongshotStart(result.map(|image| (region, image))))
                    }
                    Destination::Record => unreachable!(),
                }
            })?;
        Ok(())
    }
}

fn finish_still(
    result: Result<RgbaImage>,
    marks: &[OutputMark],
    destination: StillDestination,
    started: Instant,
) {
    match destination {
        StillDestination::File(target) => post(Event::CaptureFinished(
            result.and_then(|image| {
                let image = render_marks(image, marks)?;
                let path = save_png(&image, &target)?;
                Ok(format!("Saved to {}", path.display()))
            }),
            started,
        )),
        StillDestination::Clipboard => post(Event::CaptureClipboard(
            result.and_then(|image| render_marks(image, marks)),
            started,
        )),
        StillDestination::Editor => post(Event::EditorOpen(
            result.and_then(|image| edit_project(image, marks)),
        )),
    }
}

fn take_last_mark(marks: &VecModel<CaptureAnnotation>) -> CaptureAnnotation {
    let index = marks.row_count() - 1;
    let mark = marks.row_data(index).expect("仅在对应历史非空时提交");
    marks.remove(index);
    mark
}

fn make_project(source: Arc<RgbaImage>, marks: &[OutputMark]) -> Result<ImageProject> {
    let mut editor = Editor::new(source);
    for mark in marks {
        let geometry = &mark.geometry;
        let tool = match geometry.tool {
            CaptureTool::Rectangle => Tool::Rectangle,
            CaptureTool::Ellipse => Tool::Ellipse,
            CaptureTool::Line => Tool::Line,
            CaptureTool::Arrow => Tool::Arrow,
            CaptureTool::Highlight => Tool::Highlight,
            CaptureTool::Pen => Tool::Pen,
            CaptureTool::Text => Tool::Text,
            CaptureTool::Mosaic => Tool::Mosaic,
            CaptureTool::Select => unreachable!("框选操作不生成标注"),
        };
        let ink = geometry.ink;
        let color = format!("#{:02x}{:02x}{:02x}", ink.red(), ink.green(), ink.blue());
        let endpoints = [[geometry.x0, geometry.y0], [geometry.x1, geometry.y1]];
        let points = if geometry.tool == CaptureTool::Pen {
            mark.points.as_slice()
        } else {
            &endpoints
        };
        editor
            .annotate(tool, points, &color, geometry.stroke, &mark.text)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(editor.snapshot())
}

fn render_marks(image: RgbaImage, marks: &[OutputMark]) -> Result<RgbaImage> {
    if marks.is_empty() {
        return Ok(image);
    }
    make_project(Arc::new(image), marks)?
        .render()
        .map_err(anyhow::Error::msg)
}

fn edit_project(image: RgbaImage, marks: &[OutputMark]) -> Result<EditedCapture> {
    let source = Arc::new(image);
    let project = make_project(source.clone(), marks)?;
    let rendered = if marks.is_empty() {
        source
    } else {
        Arc::new(project.render().map_err(anyhow::Error::msg)?)
    };
    Ok((rendered, project))
}

pub(super) fn save_png(image: &RgbaImage, target: &SaveTarget) -> Result<PathBuf> {
    save_rgba(image.as_raw(), image.width(), image.height(), target)
}

#[cfg(target_os = "linux")]
pub(super) fn system_screenshot(
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Option<RgbaImage>> {
    use clippy_capture::{image_validation::decode_strict_png, project::MAX_SOURCE_PNG_BYTES};
    use io::Read;
    use std::sync::atomic::Ordering;

    let Some(path) = capture::system_screenshot()? else {
        return Ok(None);
    };
    if cancelled.load(Ordering::Acquire) {
        return Ok(None);
    }
    let metadata = path
        .metadata()
        .context("Cannot read the system screenshot")?;
    anyhow::ensure!(
        metadata.is_file() && metadata.len() <= MAX_SOURCE_PNG_BYTES as u64,
        "The system screenshot is not a regular PNG within the size limit"
    );
    let mut png = Vec::new();
    fs::File::open(path)?
        .take(MAX_SOURCE_PNG_BYTES as u64 + 1)
        .read_to_end(&mut png)?;
    let image = decode_strict_png(&png, MAX_SOURCE_PNG_BYTES, "System screenshot")
        .map_err(anyhow::Error::msg)?;
    Ok(Some(image))
}

pub(super) fn save_screens(target: SaveTarget) -> io::Result<()> {
    thread::Builder::new()
        .name("clippy-screenshot-save".into())
        .spawn(move || {
            let started = Instant::now();
            let result = capture::freeze().and_then(|outputs| {
                let mut paths = Vec::with_capacity(outputs.len());
                for screen in outputs {
                    let image = screen.pixels;
                    let saved = save_rgba(&image.rgba, image.width, image.height, &target)
                        .with_context(|| {
                            format!(
                                "Saved {} screen(s). Could not save {}",
                                paths.len(),
                                screen.name
                            )
                        })?;
                    paths.push(saved.display().to_string());
                }
                Ok(format!("Saved {}", paths.join(", ")))
            });
            post(Event::CaptureFinished(result, started));
        })?;
    Ok(())
}

fn save_rgba(rgba: &[u8], width: u32, height: u32, target: &SaveTarget) -> Result<PathBuf> {
    fs::create_dir_all(&target.directory)?;
    let stem = target.filename_stem("clippy-screenshot");
    let mut staging = tempfile::Builder::new()
        .prefix(".clippy-save-")
        .tempfile_in(&target.directory)?;
    PngEncoder::new_with_quality(
        staging.as_file_mut(),
        CompressionType::Fast,
        FilterType::Adaptive,
    )
    .write_image(rgba, width, height, ExtendedColorType::Rgba8)?;
    staging.as_file().sync_all()?;
    for candidate in filename_candidates(&stem) {
        let path = target.directory.join(candidate);
        match staging.persist_noclobber(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                staging = error.file
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    anyhow::bail!("Too many screenshots with the same filename")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn overlay_and_crop_share_one_immutable_source_buffer() {
        let source: clippy_platform::capture::FrozenPixels =
            RgbaImage::from_fn(4, 2, |x, y| Rgba([x as u8, y as u8, 32, 127])).into();
        let original = Arc::downgrade(&source.rgba);
        let frame = OverlayFrame::from(FrozenOutput {
            name: String::from("test"),
            geometry: DesktopRect {
                x: 0,
                y: 0,
                width: 4,
                height: 2,
            },
            pixels: source,
        });
        assert!(original.upgrade().is_none());
        let display = slint::Image::from_rgba8(frame.pixels.clone())
            .to_rgba8()
            .unwrap();
        assert!(std::ptr::eq(display.as_bytes(), frame.pixels.as_bytes()));
        let crop = capture::crop_rgba(
            frame.pixels.as_bytes(),
            frame.pixels.width(),
            frame.pixels.height(),
            [0.5, 0.5, 0.75, 1.0],
        )
        .unwrap();
        assert_eq!(crop.dimensions(), (1, 1));
        assert_eq!(crop.get_pixel(0, 0).0, [2, 1, 32, 127]);
    }

    #[test]
    fn commits_complete_pngs_without_overwriting_or_leaving_staging_files() {
        let temporary = tempfile::tempdir().unwrap();
        let target = SaveTarget {
            directory: temporary.path().into(),
            template: "test".into(),
        };
        let original = RgbaImage::from_pixel(3, 2, Rgba([141, 54, 77, 105]));
        let replacement = RgbaImage::from_pixel(1, 2, Rgba([50, 53, 51, 255]));
        let first = save_png(&original, &target).unwrap();
        let second = save_png(&replacement, &target).unwrap();
        assert_eq!(first.file_name().unwrap(), "test.png");
        assert_eq!(second.file_name().unwrap(), "test-2.png");
        assert_eq!(image::open(first).unwrap().into_rgba8(), original);
        assert_eq!(image::open(second).unwrap().into_rgba8(), replacement);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 2);
    }
}
