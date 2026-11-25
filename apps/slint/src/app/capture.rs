use super::{post, Event};
use crate::{CaptureMark, CaptureTool, CaptureWindow};
use anyhow::{Context, Result};
use clippy_capture::editor::{Editor, ImageProject, Tool};
use clippy_core::image_output::{filename_candidates, SaveTarget};
use clippy_platform::capture::{self, DesktopRect, FrozenOutput, Region};
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ExtendedColorType, ImageEncoder, RgbaImage,
};
use slint::winit_030::{
    winit::{
        event::WindowEvent,
        window::{Fullscreen, WindowLevel},
    },
    EventResult, WinitWindowAccessor,
};
use slint::{
    CloseRequestResponse, ComponentHandle, Model, ModelRc, PhysicalPosition, PhysicalSize,
    Rgba8Pixel, SharedPixelBuffer, VecModel,
};
use std::{fs, io, path::PathBuf, rc::Rc, sync::Arc, thread, time::Instant};

pub(super) struct View {
    window: CaptureWindow,
    frame: OverlayFrame,
}

struct OverlayFrame {
    pixels: SharedPixelBuffer<Rgba8Pixel>,
    name: String,
    geometry: DesktopRect,
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

pub(super) enum Destination {
    Clipboard,
    File(SaveTarget),
    Editor,
    Longshot,
    Record,
}

pub(super) type EditedCapture = (Arc<RgbaImage>, ImageProject);

impl Drop for View {
    fn drop(&mut self) {
        let _ = self.window.hide();
    }
}

pub(super) fn start(started: Instant) -> io::Result<()> {
    thread::Builder::new()
        .name("clippy-freeze".into())
        .spawn(move || {
            post(Event::CaptureReady {
                started,
                result: capture::freeze(),
            });
        })?;
    Ok(())
}

pub(super) fn present(
    outputs: Vec<FrozenOutput>,
    started: Instant,
    diagnostics: bool,
) -> Result<Vec<View>> {
    let single_output = outputs.len() == 1;
    outputs
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let wayland = clippy_platform::session::is_wayland();
            let window = CaptureWindow::new()?;
            let frame = OverlayFrame::from(output);
            let view = View { window, frame };
            let geometry = view.frame.geometry;
            let pixels = &view.frame.pixels;
            view.window.set_source_width(pixels.width() as i32);
            view.window.set_source_height(pixels.height() as i32);
            view.window
                .set_frame(slint::Image::from_rgba8(pixels.clone()));
            view.window
                .window()
                .set_position(PhysicalPosition::new(geometry.x, geometry.y));
            let dimensions = if wayland {
                (pixels.width(), pixels.height())
            } else {
                (geometry.width, geometry.height)
            };
            view.window
                .window()
                .set_size(PhysicalSize::new(dimensions.0, dimensions.1));
            view.window.on_cancel(|| post(Event::CaptureCancel));
            let marks = Rc::new(VecModel::default());
            view.window.set_marks(ModelRc::from(marks.clone()));
            view.window.on_add_mark({
                let marks = marks.clone();
                move |mark| marks.push(mark)
            });
            view.window.on_undo_mark({
                let marks = marks.clone();
                move || {
                    marks.remove(marks.row_count() - 1);
                }
            });
            view.window
                .on_clear_marks(move || marks.set_vec(Vec::new()));
            view.window.window().on_close_requested(|| {
                post(Event::CaptureCancel);
                CloseRequestResponse::HideWindow
            });
            view.window
                .on_confirm(move |action, left, top, right, bottom| {
                    post(Event::CaptureSelection {
                        index,
                        action,
                        bounds: [left, top, right, bottom],
                    });
                });
            let ready_name = view.frame.name.clone();
            let placement_window = view.window.as_weak();
            let mut needs_placement = wayland;
            let mut first_redraw = true;
            view.window
                .window()
                .on_winit_window_event(move |window, event| {
                    if needs_placement {
                        // Wayland 的原生窗口由事件循环建立，show 返回时可能还没有。
                        if let Some(result) = window.with_winit_window(|native| {
                            let matching = native.available_monitors().find(|monitor| {
                                monitor.name().as_deref() == Some(ready_name.as_str())
                            });
                            if matching.is_none() && !single_output {
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
                    if diagnostics && first_redraw && matches!(event, WindowEvent::RedrawRequested)
                    {
                        first_redraw = false;
                        eprintln!(
                            "capture_overlay output={ready_name} elapsed_ms={:.1}",
                            started.elapsed().as_secs_f64() * 1000.0
                        );
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
        .collect()
}

pub(super) fn output(
    mut views: Vec<View>,
    index: usize,
    bounds: [f32; 4],
    destination: Destination,
) -> io::Result<()> {
    let selected = views.swap_remove(index);
    let region = Region {
        output: selected.frame.name.clone(),
        geometry: selected.frame.geometry,
        bounds,
    };
    let marks: Vec<_> = selected.window.get_marks().iter().collect();
    let pixels = selected.frame.pixels.clone();
    drop(selected);
    views.clear();
    thread::Builder::new()
        .name("clippy-screenshot-output".into())
        .spawn(move || {
            let started = Instant::now();
            let (width, height) = (pixels.width(), pixels.height());
            if matches!(destination, Destination::Record) {
                post(Event::RecordingStart(
                    capture::pixel_rect(width, height, bounds)
                        .map(|[left, top, right, bottom]| (region, (right - left, bottom - top))),
                ));
                return;
            }
            let result = capture::crop_rgba(pixels.as_bytes(), width, height, bounds);
            match destination {
                Destination::File(target) => post(Event::CaptureFinished(
                    result.and_then(|image| {
                        let image = render_marks(image, &marks)?;
                        let path = save_png(&image, &target)?;
                        Ok(format!("Saved to {}", path.display()))
                    }),
                    started,
                )),
                Destination::Clipboard => post(Event::CaptureClipboard(
                    result.and_then(|image| render_marks(image, &marks)),
                    started,
                )),
                Destination::Editor => post(Event::EditorOpen(
                    result.and_then(|image| edit_project(image, &marks)),
                )),
                Destination::Longshot => {
                    post(Event::LongshotStart(result.map(|image| (region, image))))
                }
                Destination::Record => unreachable!(),
            }
        })?;
    Ok(())
}

fn make_project(source: Arc<RgbaImage>, marks: &[CaptureMark]) -> Result<ImageProject> {
    let mut editor = Editor::new(source);
    for mark in marks {
        let tool = match mark.tool {
            CaptureTool::Rectangle => Tool::Rectangle,
            CaptureTool::Ellipse => Tool::Ellipse,
            CaptureTool::Line => Tool::Line,
            CaptureTool::Arrow => Tool::Arrow,
            CaptureTool::Highlight => Tool::Highlight,
            CaptureTool::Select => unreachable!("框选操作不生成标注"),
        };
        let ink = mark.ink;
        let color = format!("#{:02x}{:02x}{:02x}", ink.red(), ink.green(), ink.blue());
        editor
            .annotate(
                tool,
                &[[mark.x0, mark.y0], [mark.x1, mark.y1]],
                &color,
                mark.stroke,
                "",
            )
            .map_err(anyhow::Error::msg)?;
    }
    Ok(editor.snapshot())
}

fn render_marks(image: RgbaImage, marks: &[CaptureMark]) -> Result<RgbaImage> {
    if marks.is_empty() {
        return Ok(image);
    }
    make_project(Arc::new(image), marks)?
        .render()
        .map_err(anyhow::Error::msg)
}

fn edit_project(image: RgbaImage, marks: &[CaptureMark]) -> Result<EditedCapture> {
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
