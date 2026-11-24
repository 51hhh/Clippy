use super::{post, Event};
use crate::CaptureWindow;
use anyhow::{Context, Result};
use clippy_core::image_output::{filename_candidates, SaveTarget};
use clippy_platform::capture::{self, DesktopRect, FrozenOutput, FrozenPixels, Region};
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
    CloseRequestResponse, ComponentHandle, PhysicalPosition, PhysicalSize, Rgba8Pixel,
    SharedPixelBuffer,
};
use std::{fs, io, path::PathBuf, thread, time::Instant};

pub(super) struct View {
    window: CaptureWindow,
    pixels: FrozenPixels,
    name: String,
    geometry: DesktopRect,
}

pub(super) enum Destination {
    Clipboard,
    File(SaveTarget),
    Editor,
    Longshot,
}

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
    frozen: Vec<FrozenOutput>,
    started: Instant,
    diagnostics: bool,
) -> Result<Vec<View>> {
    let single_output = frozen.len() == 1;
    frozen
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            let wayland = clippy_platform::session::is_wayland();
            let window = CaptureWindow::new()?;
            let view = View {
                window,
                pixels: output.pixels,
                name: output.name.clone(),
                geometry: output.geometry,
            };
            let geometry = output.geometry;
            let pixels = &view.pixels;
            let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
                &pixels.rgba,
                pixels.width,
                pixels.height,
            );
            view.window.set_frame(slint::Image::from_rgba8(buffer));
            view.window
                .window()
                .set_position(PhysicalPosition::new(geometry.x, geometry.y));
            let dimensions = if wayland {
                (pixels.width, pixels.height)
            } else {
                (geometry.width, geometry.height)
            };
            view.window
                .window()
                .set_size(PhysicalSize::new(dimensions.0, dimensions.1));
            view.window.on_cancel(|| post(Event::CaptureCancel));
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
            let ready_name = output.name.clone();
            let mut first_redraw = true;
            view.window.window().on_winit_window_event(move |_, event| {
                if diagnostics && first_redraw && matches!(event, WindowEvent::RedrawRequested) {
                    first_redraw = false;
                    eprintln!(
                        "capture_overlay output={ready_name} elapsed_ms={:.1}",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                EventResult::Propagate
            });
            view.window.show()?;
            if wayland {
                view.window
                    .window()
                    .with_winit_window(|native| {
                        let matching = native.available_monitors().find(|monitor| {
                            monitor.name().as_deref() == Some(output.name.as_str())
                        });
                        if matching.is_none() && !single_output {
                            anyhow::bail!("Cannot match Wayland output {}", output.name);
                        }
                        native.set_fullscreen(Some(Fullscreen::Borderless(matching)));
                        anyhow::Ok(())
                    })
                    .context("Cannot place Wayland screenshot overlay")??;
            } else {
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
    let View {
        window,
        pixels,
        name,
        geometry,
    } = &views[index];
    let region = Region {
        output: name.clone(),
        geometry: *geometry,
        bounds,
    };
    // 先隐藏所有预览再做裁剪，独立保留用户选中那一屏的冻结像素。
    let _ = window.hide();
    let pixels = FrozenPixels {
        width: pixels.width,
        height: pixels.height,
        rgba: pixels.rgba.clone(),
    };
    views.clear();
    thread::Builder::new()
        .name("clippy-screenshot-output".into())
        .spawn(move || {
            let started = Instant::now();
            let result = pixels.crop(bounds[0], bounds[1], bounds[2], bounds[3]);
            match destination {
                Destination::File(target) => post(Event::CaptureFinished(
                    result.and_then(|image| {
                        let path = save_png(&image, &target)?;
                        Ok(format!("Saved to {}", path.display()))
                    }),
                    started,
                )),
                Destination::Clipboard => post(Event::CaptureClipboard(result, started)),
                Destination::Editor => post(Event::EditorOpen(result)),
                Destination::Longshot => {
                    post(Event::LongshotStart(result.map(|image| (region, image))))
                }
            }
        })?;
    Ok(())
}

pub(super) fn save_png(image: &RgbaImage, target: &SaveTarget) -> Result<PathBuf> {
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
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgba8,
    )?;
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
