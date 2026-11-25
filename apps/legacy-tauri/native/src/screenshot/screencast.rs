use super::Rect;
use anyhow::Result;
use clippy_platform::capture::{gnome, DesktopRect, FrozenPixels as ScreencastFrame};

pub(super) fn capture_areas(areas: &[(String, Rect)]) -> Result<Vec<Result<ScreencastFrame>>> {
    let areas = areas
        .iter()
        .map(|(label, rect)| {
            (
                label.clone(),
                DesktopRect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                },
            )
        })
        .collect::<Vec<_>>();
    crate::dbus::off_async_runtime(|| gnome::capture_areas(&areas))
}

#[cfg(test)]
fn capture_monitors(connectors: &[String]) -> Result<Vec<Result<ScreencastFrame>>> {
    crate::dbus::off_async_runtime(|| gnome::capture_monitors(connectors))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真机计时，默认 `#[ignore]`：
    /// `cargo test --lib screencast_timings -- --ignored --nocapture`
    ///
    /// 要看的是两件事：**墙钟**（对比扩展那条 1.9 s 的路）和**每块屏的像素尺寸**
    /// （必须等于逻辑尺寸 × 真实缩放，糊就是糊在这个数上）。所以两块屏一起取一次，
    /// 再逐块单独取一次——用户报"多屏时更慢"时，这两个数字能直接分开会话开销和取帧开销。
    #[test]
    #[ignore = "需要真实桌面会话"]
    fn screencast_timings() {
        let Ok(monitors) =
            crate::screenshot::backends::enumerate_wayland_monitors_with_connectors()
        else {
            println!("拿不到 Wayland 显示器，跳过");
            return;
        };
        let connectors: Vec<String> = monitors
            .iter()
            .map(|(_, connector)| connector.clone())
            .collect();
        for (info, connector) in &monitors {
            println!(
                "显示器 {connector}: 逻辑 {}x{}@{},{} ×{:.4} → 期望 {}x{}",
                info.rect.width,
                info.rect.height,
                info.rect.x,
                info.rect.y,
                info.scale_factor,
                (info.rect.width as f32 * info.scale_factor).round(),
                (info.rect.height as f32 * info.scale_factor).round(),
            );
        }

        let at = std::time::Instant::now();
        let frames = capture_monitors(&connectors);
        println!(
            "一次会话取 {} 块屏: {:.1} ms",
            connectors.len(),
            at.elapsed().as_secs_f64() * 1000.0
        );
        match &frames {
            Ok(frames) => {
                for (connector, frame) in connectors.iter().zip(frames) {
                    match frame {
                        Ok(frame) => println!(
                            "  {connector}: {}x{}，{} KiB",
                            frame.width,
                            frame.height,
                            frame.rgba.len() / 1024
                        ),
                        Err(error) => println!("  {connector}: 失败: {error:#}"),
                    }
                }
            }
            Err(error) => println!("  失败: {error:#}"),
        }

        for connector in &connectors {
            let at = std::time::Instant::now();
            let single = capture_monitors(std::slice::from_ref(connector));
            println!(
                "单独取 {connector}: {:.1} ms → {:?}",
                at.elapsed().as_secs_f64() * 1000.0,
                single.map(|frames| frames
                    .iter()
                    .map(|frame| {
                        frame
                            .as_ref()
                            .map(|frame| (frame.width, frame.height))
                            .map_err(|error| format!("{error:#}"))
                    })
                    .collect::<Vec<_>>())
            );
        }

        let areas = monitors
            .iter()
            .map(|(info, connector)| (connector.clone(), info.rect))
            .collect::<Vec<_>>();
        let at = std::time::Instant::now();
        let frames = capture_areas(&areas);
        println!(
            "RecordArea 一次会话取 {} 块屏: {:.1} ms",
            areas.len(),
            at.elapsed().as_secs_f64() * 1000.0
        );
        match frames {
            Ok(frames) => {
                for ((label, _), frame) in areas.iter().zip(frames) {
                    match frame {
                        Ok(frame) => println!("  {label}: {}x{}", frame.width, frame.height),
                        Err(error) => println!("  {label}: 失败: {error:#}"),
                    }
                }
            }
            Err(error) => println!("  失败: {error:#}"),
        }
    }

    /// 把当前每块屏的原生画面存成 PNG，默认 `#[ignore]`：
    /// `CLIPPY_FRAME_DUMP=/tmp cargo test --lib dump_screencast_frames -- --ignored --nocapture`
    ///
    /// 为什么要有它：**"屏幕上看起来糊"这类问题只能拿设备像素去比。** 有了这个 dump，
    /// 就能把某个窗口在屏幕上的实际成像和它的图片源逐像素对照（例如贴图窗口有没有被
    /// WebKit 重采样过），而不是靠肉眼争论。落地路径由 `CLIPPY_FRAME_DUMP` 指定，
    /// 不设就跳过——这条测试会写文件，不能在别人不知情时往磁盘上放几十兆。
    #[test]
    #[ignore = "需要真实桌面会话"]
    fn dump_screencast_frames() {
        let Ok(directory) = std::env::var("CLIPPY_FRAME_DUMP") else {
            println!("没设 CLIPPY_FRAME_DUMP，跳过");
            return;
        };
        let Ok(monitors) =
            crate::screenshot::backends::enumerate_wayland_monitors_with_connectors()
        else {
            println!("拿不到 Wayland 显示器，跳过");
            return;
        };
        let areas: Vec<(String, Rect)> = monitors
            .iter()
            .map(|(info, connector)| (connector.clone(), info.rect))
            .collect();
        let frames = capture_areas(&areas).expect("取流失败");
        for ((connector, _), frame) in areas.iter().zip(&frames) {
            let frame = frame.as_ref().expect("取流失败");
            let png = crate::screenshot::encode_png(&frame.rgba, frame.width, frame.height)
                .expect("编码失败");
            let path = std::path::Path::new(&directory).join(format!("frame-{connector}.png"));
            std::fs::write(&path, png).expect("写文件失败");
            println!("{} {}x{}", path.display(), frame.width, frame.height);
        }
    }
}
