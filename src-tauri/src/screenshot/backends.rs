#[cfg(target_os = "linux")]
use super::geometry_check::{
    classify_stage, crop_coverage_ratio, desktop_max_scale_factor, find_mirror_sources,
    verify_crop_not_clamped, verify_crops_do_not_overlap, verify_frame_isotropy, StageClass,
};
#[cfg(any(test, target_os = "linux"))]
use super::ImageRect;
use super::{DesktopBounds, FrozenFrame, MonitorInfo, Rect};
#[cfg(any(test, target_os = "linux"))]
use anyhow::anyhow;
use anyhow::{bail, Context, Result};
#[cfg(target_os = "linux")]
use image::RgbaImage;
use std::sync::Arc;
use xcap::Monitor;

/// 逐屏取画面时一块屏的画面：像素宽高 + RGBA8（行优先、行内无填充）。
#[cfg(target_os = "linux")]
type MonitorTile = (u32, u32, Arc<[u8]>);

/// 一个用完即删的截图文件。三个后端都用它：GNOME Shell / 扩展写在私有临时目录，
/// Portal 则把文件塞进用户的图片目录，任由它留着会把相册塞满。
#[cfg(target_os = "linux")]
pub(super) struct TemporaryScreenshotFile {
    path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl TemporaryScreenshotFile {
    pub(super) fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub(super) fn replace_path(&mut self, path: std::path::PathBuf) {
        self.remove_current();
        self.path = path;
    }

    fn remove_current(&self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("删除临时截图失败 {}: {error}", self.path.display());
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl Drop for TemporaryScreenshotFile {
    fn drop(&mut self) {
        self.remove_current();
    }
}

/// 冻结帧的后端链。顺序不是随手排的：
///
/// 1. **Mutter 的 PipeWire 屏幕流**（`screencast.rs`）——GNOME 上又快又清楚的那条路：
///    原生像素、不编 PNG，实测两块屏合计 190 ms（旧路径 1900 ms）。同一个用户直接可调，
///    不经 Portal、不弹对话框。不是 GNOME 时第一个 D-Bus 调用就失败，几毫秒退到下一条。
/// 2. **自带的 GNOME Shell 扩展，逐屏原生截取**——不弹对话框、不闪白、不往用户图片目录里
///    落文件，每块屏也是原生像素（见 `capture_all_shell_extension_monitor_areas`）。
///    但画面要经 gnome-shell 的 PNG 编码器，4K 一块屏就要 1.7 秒，所以只是兜底。
///    - 协议低于 v4（装了新版还没注销）或逐屏失败时，同一个扩展退到**整屏舞台图**：
///      画面可用，但混合缩放的多屏上低缩放那块会被上采样，偏糊。
/// 3. **wlroots（libwayshot）**——sway/Hyprland 系合成器的原生路径。
/// 4. **XDG Portal（只用非交互）**——KDE 等实现里最可靠的兜底。
///    **绝不用 interactive 模式**：那个模式在 GNOME 上就是系统自带的截图界面，
///    用户按 Clippy 的快捷键却看到系统 UI，还得再选一次区域，比失败更糟。
/// 5. **org.gnome.Shell.Screenshot**——GNOME 现在白名单外一律拒绝，留着只为老版本。
/// 6. **xcap/XRandR**——X11 会话的正路，Wayland 下只能看到 XWayland。
#[cfg(target_os = "linux")]
pub(super) fn capture_all_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    if is_wayland_session() {
        match capture_all_screencast_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::info!("Mutter PipeWire 取流不可用，回退到扩展逐屏截图: {e:#}"),
        }

        match capture_all_shell_extension_monitor_areas() {
            Ok(result) => return Ok(result),
            Err(e) => log::info!("扩展逐屏原生截图不可用，回退到整屏舞台图: {e:#}"),
        }

        match capture_all_shell_extension_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::info!("GNOME Shell 扩展截图不可用，回退到 wlroots: {e:#}"),
        }

        match capture_all_wayland_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("Wayland wlroots 截图失败，回退到 Portal: {e:#}"),
        }

        match capture_all_portal_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("XDG Portal 截图失败，回退到 GNOME Shell: {e:#}"),
        }

        match capture_all_gnome_shell_monitors() {
            Ok(result) => return Ok(result),
            Err(e) => log::warn!("GNOME Shell 截图失败，回退到 xcap: {e:#}"),
        }
    }

    capture_all_xcap_monitors()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn capture_all_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    #[cfg(target_os = "macos")]
    if !crate::platform::macos_screen_capture_trusted()
        && !crate::platform::request_macos_screen_capture_permission()
    {
        bail!("macOS 屏幕录制权限尚未授予");
    }
    capture_all_xcap_monitors()
}

fn capture_all_xcap_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = Monitor::all().context("无法枚举显示器")?;
    let mut infos = Vec::new();
    let mut frames = Vec::new();

    for mon in monitors.iter() {
        let mut info = monitor_info(mon)?;
        let img = mon.capture_image().context("无法捕获显示器")?;
        let frame_width = img.width();
        let frame_height = img.height();
        info.rect = normalize_monitor_geometry(info.rect, info.scale_factor, frame_width);
        frames.push(FrozenFrame {
            monitor_id: info.id,
            rgba: Arc::from(img.into_raw()),
            width: frame_width,
            height: frame_height,
            scale_factor: info.scale_factor,
        });
        infos.push(info);
    }

    Ok((infos, frames))
}

/// 用冻结帧的真实像素宽度反推逻辑尺寸，修正 xcap 报出的显示器几何。
///
/// `pixel_width` 与 `scale_factor` 必须来自**同一个坐标系**。逐屏抓图时
/// （`capture_all_xcap_monitors`）`img.width()` 就是这块屏自己的物理宽度，除数是这块屏自己的
/// `scale_factor`；切整张舞台图时（`split_portal_screenshot`）裁剪宽度是"逻辑宽 × 桌面最大
/// 缩放"，除数只能是 `geometry_check::desktop_max_scale_factor`，而且**只在几何确实是物理味
/// 时才该调用**——该走哪条路由 `geometry_check::classify_stage` 判定，不要再靠这里的 1 像素
/// 护栏碰运气（混合缩放的多屏正好能绕过它）。
///
/// xcap 在 Linux 上把 RandR 尺寸除以自己探测的 `scale_factor` 当逻辑尺寸，但 GNOME 给
/// XWayland 的 X screen 是"逻辑尺寸 × 整数倍"，与真实缩放并不相等：实测 1920x1200 的桌面
/// 被报成 2880x1800（RandR 3840x2400 ÷ 1.333）。覆盖层按这个尺寸开窗就会既错位又错缩放，
/// 而冻结帧一定是物理像素，因此 `物理像素 ÷ scale_factor` 才是可信的逻辑尺寸。
/// 原点按同一比例折算——xcap 对 x/y 用的是同一个除数，比例是一致的。
pub(super) fn normalize_monitor_geometry(rect: Rect, scale_factor: f32, pixel_width: u32) -> Rect {
    if !scale_factor.is_finite() || scale_factor <= 0.0 || pixel_width == 0 || rect.width == 0 {
        return rect;
    }
    let logical_width = (pixel_width as f32 / scale_factor).round();
    if !logical_width.is_finite() || logical_width < 1.0 {
        return rect;
    }
    // 1px 以内的差异是取整噪声，不值得改动几何。
    let ratio = rect.width as f32 / logical_width;
    if (rect.width as f32 - logical_width).abs() <= 1.0 || !ratio.is_finite() || ratio <= 0.0 {
        return rect;
    }
    log::info!(
        "显示器几何被修正: {}x{} -> {}x{}（缩放 {scale_factor}，帧宽 {pixel_width}）",
        rect.width,
        rect.height,
        logical_width as u32,
        (rect.height as f32 / ratio).round() as u32
    );
    Rect {
        x: (rect.x as f32 / ratio).round() as i32,
        y: (rect.y as f32 / ratio).round() as i32,
        width: logical_width as u32,
        height: (rect.height as f32 / ratio).round().max(1.0) as u32,
    }
}

#[cfg(target_os = "linux")]
fn capture_all_wayland_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let conn = wayland_connection()?;
    let outputs = conn.get_all_outputs().to_vec();
    if outputs.is_empty() {
        bail!("Wayland compositor 未报告输出");
    }

    let mut infos = Vec::with_capacity(outputs.len());
    let mut frames = Vec::with_capacity(outputs.len());
    for (index, output) in outputs.iter().enumerate() {
        let info = monitor_info_from_wayland_output(index, output);
        let image = conn
            .screenshot_single_output(output, false)
            .with_context(|| format!("无法捕获 Wayland 输出 {}", output.name))?;
        // **旋转必须在这里补上。** `screenshot_single_output` 直接把 frame copy 转成图，
        // 一个字都不转（会转的是 libwayshot 自己的 `screenshot_outputs` 合成路径）。
        // 于是竖屏拿到的是面板原始朝向的横向像素：覆盖层里画面躺倒，帧宽高也和逻辑矩形
        // 反着，选区坐标全错。
        let rgba_image = apply_output_transform(image.to_rgba8(), output.transform);
        frames.push(FrozenFrame {
            monitor_id: info.id,
            width: rgba_image.width(),
            height: rgba_image.height(),
            rgba: Arc::from(rgba_image.into_raw()),
            scale_factor: info.scale_factor,
        });
        infos.push(info);
    }

    Ok((infos, frames))
}

/// 逐屏走扩展的 `CaptureArea`：每块屏拿到的都是**它自己面板的原生像素**，
/// 而且是原始 RGBA，两头都不经过 PNG。
///
/// **理由一是画质**，根因在 Mutter 里：整屏截图的尺寸由
/// `clutter_stage_get_capture_final_size` 算成"矩形 × max(与该矩形相交的各视图的缩放)"，
/// 而整个 stage 的矩形跟每块屏都相交，于是整张图都按全桌面最大的那个缩放渲染。
/// 实测本机（eDP 原生 2560x1600、逻辑 1920x1200、缩放 1.3333；外接 4K 缩放 1.5）：
/// 舞台图 6720x2412 里 eDP 那块是 2880x1800 —— 逻辑 × 1.5，比原生**多出 1.125 倍**，
/// 全是插值出来的像素。糊就糊在这最开始的一步，后面裁剪、传输、导出一个环节都救不回来。
/// 现在像素尺寸由我们传给扩展的 `scale` 决定，与相交视图无关。
///
/// **速度这一头这条路没解决。** 整条链路上最贵的一步是 gnome-shell 编 PNG：同一批像素、
/// 两种处理的对照实验（拍下来解开，再用同一个 gdk-pixbuf 重编一次）显示 4K 那块屏
/// 1704 ms 里有 **1607 ms（94%）是 deflate**，合成器绘制 + 读回只有约 100 ms。
/// 扩展里"改成 `Cogl.Texture.get_data` 落原始字节"这条路在 GJS 上不可能成立
/// （没有长度标注的 `array<uint8>` 会被复制后释放，见 `screencast.rs` 文件头与
/// docs/capture-linux.md §3.1），所以速度只能靠上面那条 PipeWire 路，这里是画质兜底。
///
/// **只在几何来自 Wayland 输出时才走**：区域坐标必须是 stage 的逻辑像素，而 xcap 在
/// XWayland 上报的是"逻辑 × 整数倍"的 X screen 尺寸（见 `normalize_monitor_geometry`），
/// 拿它当区域会截到别处去。所以这里不像下面那条路那样退到 xcap 几何，直接失败、
/// 让链路退回整屏舞台图——那条路有 `classify_stage` 兜着几何的坐标空间。
#[cfg(target_os = "linux")]
fn capture_all_shell_extension_monitor_areas() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors().context("逐屏扩展截图无法枚举 Wayland 几何")?;
    let tiles = capture_shell_extension_tiles(&monitors)?;
    let ordered: Vec<&MonitorTile> = tiles.iter().collect();

    assemble_native_frames(
        &monitors,
        &ordered,
        &format!("逐屏原生截取（{} 块屏）", tiles.len()),
    )
}

/// 只为给定显示器取扩展原始像素。PipeWire 部分成功时会传入缺帧子集，避免把已经在
/// 几十毫秒内拿到的屏幕再截一遍；完整扩展兜底也复用同一实现。
#[cfg(target_os = "linux")]
fn capture_shell_extension_tiles(monitors: &[MonitorInfo]) -> Result<Vec<MonitorTile>> {
    let (areas, assignment) = dedupe_monitor_areas(monitors);
    let captures =
        crate::capture::shell_extension_area_captures(&areas).map_err(|error| anyhow!(error))?;
    // 包成 TemporaryScreenshotFile：读成功也好、失败也好，出了作用域文件一定被删。
    let shots: Vec<(TemporaryScreenshotFile, crate::capture::AreaCapture)> = captures
        .into_iter()
        .map(|capture| {
            (
                TemporaryScreenshotFile::new(capture.path().to_path_buf()),
                capture,
            )
        })
        .collect();

    // 逐屏并行：原始像素只是一次 tmpfs 读，退回 PNG 的那块屏才需要解码（8 Mpx 几十毫秒），
    // 而这几块屏之间毫无依赖。
    let loaded: Vec<Result<MonitorTile>> = std::thread::scope(|scope| {
        let handles: Vec<_> = shots
            .iter()
            .map(|(_, capture)| scope.spawn(move || load_area_tile(capture)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(anyhow!("读取逐屏画面的线程 panic")))
            })
            .collect()
    });
    let unique_tiles = loaded.into_iter().collect::<Result<Vec<_>>>()?;
    // 去重表还原成"每块屏一块画面"：镜像的两块屏共享同一个 Arc，不复制像素。
    Ok(assignment
        .iter()
        .map(|&index| {
            let (width, height, rgba) = &unique_tiles[index];
            (*width, *height, Arc::clone(rgba))
        })
        .collect())
}

/// GNOME Wayland 的首选路径：从 Mutter 直接拿 PipeWire 视频流。
///
/// 快是因为**没有 PNG**（旧路径 94% 的时间在 deflate），清楚是因为每块屏都按自己的缩放
/// 单独取流、拿到的就是面板原生像素。两件事同一个改动，细节与实测数字见 `screencast.rs`。
///
/// **只在几何来自 Wayland 输出时才走**，理由和下面逐屏那条路一样：`RecordMonitor` 认的是
/// 连接器名（`eDP-1`），只有 Wayland 输出枚举给得出，xcap 那边压根没有这个字段。
#[cfg(target_os = "linux")]
fn capture_all_screencast_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors_with_connectors()
        .context("PipeWire 取流无法枚举 Wayland 几何")?;
    let connectors: Vec<String> = monitors
        .iter()
        .map(|(_, connector)| connector.clone())
        .collect();
    if let Some(index) = connectors.iter().position(|name| name.is_empty()) {
        bail!("第 {index} 块 Wayland 输出没有连接器名，RecordMonitor 无从指定显示器");
    }

    let captured = super::screencast::capture_monitors(&connectors)?;
    let infos: Vec<MonitorInfo> = monitors.into_iter().map(|(info, _)| info).collect();
    let mut tiles: Vec<Option<MonitorTile>> = Vec::with_capacity(captured.len());
    let mut missing_indices = Vec::new();
    for (index, frame) in captured.into_iter().enumerate() {
        match frame {
            Ok(frame) => tiles.push(Some((frame.width, frame.height, frame.rgba))),
            Err(error) => {
                log::info!(
                    "显示器 {} 的 PipeWire 首帧不可用，单屏回退到扩展原始像素: {error:#}",
                    connectors[index]
                );
                tiles.push(None);
                missing_indices.push(index);
            }
        }
    }
    if !missing_indices.is_empty() {
        let missing_monitors: Vec<MonitorInfo> = missing_indices
            .iter()
            .map(|&index| infos[index].clone())
            .collect();
        let fallback = capture_shell_extension_tiles(&missing_monitors)?;
        for (index, tile) in missing_indices.into_iter().zip(fallback) {
            tiles[index] = Some(tile);
        }
    }
    let tiles: Vec<MonitorTile> = tiles
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .context("截图后仍有显示器缺少画面")?;
    let ordered: Vec<&MonitorTile> = tiles.iter().collect();

    assemble_native_frames(&infos, &ordered, "逐屏 PipeWire 取流（缺帧单屏兜底）")
}

/// 把"每块屏一块原生画面"装成冻结帧：校验尺寸、按实际帧重算缩放、跑 I3 自检、留一行摘要。
///
/// 逐屏 PipeWire 与逐屏扩展截图共用它。两条路的区别只在画面怎么来的，之后的几何处理必须
/// 一模一样——分成两份写，改一条忘一条就会出现"换了后端选区就错位"这种最难查的问题。
/// `tiles[i]` 对应 `monitors[i]`。
#[cfg(target_os = "linux")]
fn assemble_native_frames(
    monitors: &[MonitorInfo],
    tiles: &[&MonitorTile],
    source: &str,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    if monitors.len() != tiles.len() {
        bail!("{} 块屏的几何配了 {} 块画面", monitors.len(), tiles.len());
    }
    let mut infos = Vec::with_capacity(monitors.len());
    let mut frames = Vec::with_capacity(monitors.len());
    for (monitor, (width, height, rgba)) in monitors.iter().zip(tiles.iter()) {
        // 帧只可能等于或大于逻辑尺寸（缩放 ≥ 1）。小于说明区域被 Clutter 钳过、
        // 或者几何是热插拔前的陈数据——这种帧铺到覆盖层上就是错位的画面，
        // 宁可整体退回整屏那条路，它有 `classify_stage` 与 I1~I3 兜着。
        if *width < monitor.rect.width || *height < monitor.rect.height {
            bail!(
                "显示器 {} 的逐屏画面 {width}x{height} 小于逻辑尺寸 {}x{}",
                monitor.id,
                monitor.rect.width,
                monitor.rect.height
            );
        }
        let frame = ImageRect {
            x: 0,
            y: 0,
            width: *width,
            height: *height,
        };
        let mut adjusted = monitor.clone();
        // 缩放按**实际拿到的帧**算，而不是信 libwayshot 从物理尺寸推出来的那个：
        // 前端的 `scale = logicalWidth / pixelWidth` 必须和这个数一致，否则选区错位。
        adjusted.scale_factor = portal_frame_scale_factor(&adjusted, frame);
        // 不变量 I3 在这条路上同样有意义：帧和几何必须同向。响了说明几何是改分辨率/
        // 热插拔之前的陈数据。画面照用（用户至少截得到东西），但要留下能查的记录。
        let anisotropy = verify_frame_isotropy(monitor.rect.width, monitor.rect.height, frame);
        if anisotropy > 0.0 {
            log::error!(
                "截图几何自检未通过：I3 显示器 {} 的帧缩放两个方向不一致（差 {anisotropy:.4}）：\
                 逻辑 {}x{}，帧 {width}x{height}",
                monitor.id,
                monitor.rect.width,
                monitor.rect.height,
            );
        }
        frames.push(FrozenFrame {
            monitor_id: adjusted.id,
            rgba: Arc::clone(rgba),
            width: *width,
            height: *height,
            scale_factor: adjusted.scale_factor,
        });
        infos.push(adjusted);
    }

    // 和整屏那条路一样每次留一行几何摘要——排障从这一行开始。不含像素与窗口标题。
    log::info!(
        "截图几何：{} 块屏{source}；{}",
        monitors.len(),
        infos
            .iter()
            .zip(frames.iter())
            .map(|(info, frame)| format!(
                "#{}@{},{} {}x{}×{:.4}→{}x{}",
                info.id,
                info.rect.x,
                info.rect.y,
                info.rect.width,
                info.rect.height,
                info.scale_factor,
                frame.width,
                frame.height,
            ))
            .collect::<Vec<_>>()
            .join("，")
    );
    Ok((infos, frames))
}

/// 把扩展交回来的一块屏读成 RGBA8。原始像素只是一次 tmpfs 读，PNG 那条路才解码。
#[cfg(target_os = "linux")]
pub(super) fn load_area_tile(capture: &crate::capture::AreaCapture) -> Result<MonitorTile> {
    let path = capture.path();
    let bytes =
        std::fs::read(path).with_context(|| format!("无法读取扩展逐屏画面 {}", path.display()))?;
    match capture {
        crate::capture::AreaCapture::Png { .. } => {
            let image = image::load_from_memory(&bytes)
                .context("无法解码扩展逐屏截图")?
                .to_rgba8();
            Ok((image.width(), image.height(), Arc::from(image.into_raw())))
        }
        &crate::capture::AreaCapture::Raw {
            width,
            height,
            stride,
            ..
        } => {
            let row = width as usize * 4;
            let rows = height as usize;
            // 最后一行不需要行内填充，所以下限是 stride × (行数 − 1) + 一行的有效字节。
            let minimum = stride * rows.saturating_sub(1) + row;
            if bytes.len() < minimum {
                bail!(
                    "扩展逐屏画面 {} 只有 {} 字节，装不下 {width}x{height}（stride {stride}）",
                    path.display(),
                    bytes.len()
                );
            }
            // 常态：stride 正好是一行，扩展写下来的字节就是我们要的那块内存，
            // 直接交出去——一次 8 Mpx 的重排也要十几毫秒，白花。
            if stride == row && bytes.len() == row * rows {
                return Ok((width, height, Arc::from(bytes)));
            }
            let mut packed = Vec::with_capacity(row * rows);
            for y in 0..rows {
                let start = y * stride;
                packed.extend_from_slice(&bytes[start..start + row]);
            }
            Ok((width, height, Arc::from(packed)))
        }
    }
}

/// 逐屏取画面前把**逻辑矩形与缩放都相同**的屏并成一次请求，返回去重后的区域
/// 与"第 i 块屏用第几个区域"的对照表。
///
/// 镜像/投影就是这个形态：两块屏共用同一个逻辑矩形，取出来的像素也一模一样，
/// 发两次同样的请求只是让用户多等一次读回。缩放必须进比较键——它决定像素尺寸，
/// 同一个矩形按两种缩放取出来根本不是同一张画面。
#[cfg(target_os = "linux")]
pub(super) fn dedupe_monitor_areas(
    monitors: &[MonitorInfo],
) -> (Vec<crate::capture::CaptureArea>, Vec<usize>) {
    let mut areas: Vec<crate::capture::CaptureArea> = Vec::new();
    let mut assignment = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let area = crate::capture::CaptureArea {
            x: monitor.rect.x,
            y: monitor.rect.y,
            width: monitor.rect.width,
            height: monitor.rect.height,
            scale: f64::from(monitor.scale_factor),
        };
        let found = areas.iter().position(|candidate| *candidate == area);
        assignment.push(match found {
            Some(index) => index,
            None => {
                areas.push(area);
                areas.len() - 1
            }
        });
    }
    (areas, assignment)
}

/// 走自带 GNOME Shell 扩展的整屏截图。几何仍旧自己枚举——扩展只负责画面。
#[cfg(target_os = "linux")]
fn capture_all_shell_extension_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let path = crate::capture::shell_extension_screenshot().map_err(|error| anyhow!(error))?;
    // 包成 TemporaryScreenshotFile：读成功也好、解码失败也好，出了作用域文件一定被删。
    let screenshot = TemporaryScreenshotFile::new(path);

    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("扩展截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举扩展截图显示器")?;

    let bytes = std::fs::read(screenshot.path())
        .with_context(|| format!("无法读取扩展截图 {}", screenshot.path().display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码扩展截图")?
        .to_rgba8();

    split_portal_screenshot(monitors, image)
}

#[cfg(target_os = "linux")]
fn capture_all_portal_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("Portal 截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举 Portal 截图显示器")?;

    // 只用非交互模式。interactive=true 在 GNOME 上就是系统自带的截图界面，
    // 顶掉 Clippy 自己的覆盖层，属于比失败更糟的结果。
    let screenshot = request_portal_screenshot().context("无法请求 Portal 截图")?;
    let path = portal_screenshot_uri_to_path(screenshot.uri().as_str())?;
    // xdg-desktop-portal-gnome 把非交互截图存进用户的图片目录（实测
    // ~/Pictures/Screenshot-N.png），一次截图留一份几百 KB 的垃圾。返回的文件按
    // Portal 约定归调用方处置，读完就删——不删的话用户的相册会被冻结帧塞满。
    let _cleanup = TemporaryScreenshotFile::new(path.clone());
    let bytes =
        std::fs::read(&path).with_context(|| format!("无法读取 Portal 截图 {}", path.display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码 Portal 截图")?
        .to_rgba8();

    split_portal_screenshot(monitors, image)
}

#[cfg(target_os = "linux")]
fn capture_all_gnome_shell_monitors() -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let monitors = enumerate_wayland_monitors()
        .or_else(|e| {
            log::warn!("GNOME Shell 截图无法复用 Wayland 几何，尝试 xcap 几何: {e:#}");
            enumerate_xcap_monitors()
        })
        .context("无法枚举 GNOME Shell 截图显示器")?;

    let screenshot = request_gnome_shell_screenshot().context("无法请求 GNOME Shell 截图")?;
    let bytes = std::fs::read(screenshot.path())
        .with_context(|| format!("无法读取 GNOME Shell 截图 {}", screenshot.path().display()))?;
    let image = image::load_from_memory(&bytes)
        .context("无法解码 GNOME Shell 截图")?
        .to_rgba8();

    split_portal_screenshot(monitors, image)
}

/// 诊断专用：只问出舞台图的**尺寸**，一个像素都不解码。
///
/// 报障要判断的是"几何和舞台图对不对得上"，而那只需要两个整数。
/// 用 `image_dimensions` 读 PNG 头，文件仍旧包在 `TemporaryScreenshotFile` 里、
/// 出了作用域一定删——诊断绝不能在用户的图片目录里留下一整屏画面。
///
/// 只试**会产出整张舞台图**的两条路。wlroots（libwayshot）是逐输出抓图的，
/// 压根不走切分那条路，所以它不在这里，也不该在这里假装有个"舞台图"。
#[cfg(target_os = "linux")]
pub(super) fn probe_stage_image_size() -> Result<(&'static str, u32, u32)> {
    let mut reasons = Vec::new();

    match crate::capture::shell_extension_screenshot() {
        Ok(path) => {
            let screenshot = TemporaryScreenshotFile::new(path);
            let (width, height) = image::image_dimensions(screenshot.path())
                .with_context(|| format!("读不出 {} 的尺寸", screenshot.path().display()))?;
            return Ok(("gnome-shell-extension", width, height));
        }
        Err(error) => reasons.push(format!("gnome-shell-extension: {error}")),
    }

    match request_portal_screenshot() {
        Ok(screenshot) => {
            let path = portal_screenshot_uri_to_path(screenshot.uri().as_str())?;
            let screenshot = TemporaryScreenshotFile::new(path);
            let (width, height) = image::image_dimensions(screenshot.path())
                .with_context(|| format!("读不出 {} 的尺寸", screenshot.path().display()))?;
            return Ok(("portal", width, height));
        }
        Err(error) => reasons.push(format!("portal: {error:#}")),
    }

    bail!("拿不到整张舞台图（{}）", reasons.join("；"))
}

pub(super) fn enumerate_xcap_monitors() -> Result<Vec<MonitorInfo>> {
    let monitors = Monitor::all().context("无法枚举显示器")?;
    monitors.iter().map(monitor_info).collect()
}

#[cfg(target_os = "linux")]
pub(super) fn enumerate_wayland_monitors() -> Result<Vec<MonitorInfo>> {
    Ok(enumerate_wayland_monitors_with_connectors()?
        .into_iter()
        .map(|(info, _)| info)
        .collect())
}

/// 同一次枚举，额外带上**连接器名**（`eDP-1`、`HDMI-1`……）。
///
/// `org.gnome.Mutter.ScreenCast.Session.RecordMonitor` 只认这个字符串，而 `MonitorInfo`
/// 里只有一个哈希出来的 id（FNV over `output.name`，为的是热插拔后仍然稳定），从 id 反推
/// 不回来。几何仍旧只有这一个来源：两个函数共用一遍枚举，免得"取流用一份几何、覆盖层用
/// 另一份"这种分叉。
#[cfg(target_os = "linux")]
pub(super) fn enumerate_wayland_monitors_with_connectors() -> Result<Vec<(MonitorInfo, String)>> {
    let conn = wayland_connection()?;
    let monitors: Vec<_> = conn
        .get_all_outputs()
        .iter()
        .enumerate()
        .map(|(index, output)| {
            (
                monitor_info_from_wayland_output(index, output),
                output.name.clone(),
            )
        })
        .collect();

    if monitors.is_empty() {
        bail!("Wayland compositor 未报告输出");
    }

    Ok(monitors)
}

/// 非交互 Portal 截图。
///
/// 已知会失败的一种情况，不要再花时间查：GNOME Wayland 上 xdg-desktop-portal 首次
/// 非交互截图要弹一个系统授权对话框，而 gnome-shell 只允许**当前聚焦的应用**弹它
/// （"Only the focused app is allowed to show a system access dialog"）。截图由全局
/// 快捷键触发，那一刻 Clippy 没有窗口聚焦，于是对话框弹不出来、请求直接失败。
/// GNOME 上的正解是走自带扩展（见 `capture_all_shell_extension_monitors`），
/// 而不是退到 interactive 模式——那就是系统自带的截图 UI。
#[cfg(target_os = "linux")]
fn request_portal_screenshot() -> Result<ashpd::desktop::screenshot::Screenshot> {
    let request = async {
        ashpd::desktop::screenshot::Screenshot::request()
            .interactive(false)
            .modal(false)
            .send()
            .await?
            .response()
    };

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(request).context("Portal 截图请求失败"),
        Err(_) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("无法创建 Portal 截图 runtime")?;
            runtime.block_on(request).context("Portal 截图请求失败")
        }
    }
}

#[cfg(target_os = "linux")]
fn request_gnome_shell_screenshot() -> Result<TemporaryScreenshotFile> {
    let directory = std::env::temp_dir().join("clippy");
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建 {}", directory.display()))?;
    crate::private_files::restrict_directory(&directory)
        .with_context(|| format!("无法收紧 {} 的目录权限", directory.display()))?;
    let path = directory.join(format!("gnome-shell-screenshot-{}.png", unique_suffix()));
    let mut screenshot = TemporaryScreenshotFile::new(path);

    let filename = screenshot.path().to_string_lossy().to_string();
    let (success, used_filename): (bool, String) = crate::dbus::call(
        "org.gnome.Shell.Screenshot",
        "/org/gnome/Shell/Screenshot",
        "org.gnome.Shell.Screenshot",
        "Screenshot",
        &(false, false, filename.as_str()),
    )
    .context("GNOME Shell Screenshot 调用失败")?;
    if !success {
        bail!("GNOME Shell Screenshot 返回 success=false");
    }

    let used_path = if used_filename.is_empty() {
        screenshot.path().to_path_buf()
    } else {
        std::path::PathBuf::from(used_filename)
    };
    validate_gnome_shell_screenshot_path(&directory, &used_path)?;
    if used_path != screenshot.path() {
        screenshot.replace_path(used_path);
    }
    if !screenshot.path().exists() {
        bail!(
            "GNOME Shell Screenshot 返回 {}，但文件不存在",
            screenshot.path().display()
        );
    }
    crate::private_files::restrict_file(screenshot.path()).with_context(|| {
        format!(
            "无法收紧 GNOME Shell 临时截图权限 {}",
            screenshot.path().display()
        )
    })?;

    Ok(screenshot)
}

#[cfg(target_os = "linux")]
pub(super) fn validate_gnome_shell_screenshot_path(
    directory: &std::path::Path,
    path: &std::path::Path,
) -> Result<()> {
    if path.parent() != Some(directory) {
        bail!("GNOME Shell Screenshot 返回的文件不在私有临时目录中");
    }
    Ok(())
}

/// 舞台图切分的**几何**部分：一块屏对应一个 [`StageTile`]。
///
/// 和 [`split_portal_screenshot`] 分开是有原因的：这里一个像素都不碰，所以
/// **诊断工具和 fixture 测试可以直接驱动真正在跑的这份代码**，不用凑一张几十兆的假图，
/// 也不用在测试里抄一遍同样的算式（抄的那份永远只能证明抄对了）。
#[cfg(target_os = "linux")]
pub(super) fn plan_stage_split(
    monitors: &[MonitorInfo],
    image_width: u32,
    image_height: u32,
) -> Result<StageSplitPlan> {
    if monitors.is_empty() {
        bail!("Portal 截图没有可映射显示器");
    }
    if image_width == 0 || image_height == 0 {
        bail!("Portal 截图为空");
    }

    let desktop = monitor_union(monitors)?;
    let scale_x = image_width as f32 / desktop.width as f32;
    let scale_y = image_height as f32 / desktop.height as f32;
    if !scale_x.is_finite() || scale_x <= 0.0 || !scale_y.is_finite() || scale_y <= 0.0 {
        bail!("Portal 截图缩放无效: {scale_x}x{scale_y}");
    }

    let mut warnings = Vec::new();

    // **不变量 I1：先判定几何处于哪个坐标空间，再决定要不要修正。** 以前是无条件调用
    // 修正函数、靠"差值 ≤ 1 像素就提前返回"当护栏，混合缩放的多屏正好能绕过它
    // （见 `geometry_check`）。
    let stage = classify_stage(
        monitors,
        desktop.width,
        desktop.height,
        image_width,
        image_height,
    );
    let repair_divisor = match stage {
        // 几何可信，一个字都不改。
        StageClass::Logical { .. } => None,
        // xcap 的物理味几何：按整台桌面的最大缩放反推逻辑尺寸。
        StageClass::Physical => Some(desktop_max_scale_factor(monitors)),
        // 既不像逻辑也不像物理：这时候任何修正都是在错上加错，宁可原样往下走，
        // 但必须留下能查的记录——覆盖层错位的根因往往就在这里。
        StageClass::Unknown {
            stage_scale,
            max_scale,
        } => {
            warnings.push(format!(
                "I1 舞台图 {image_width}x{image_height} ÷ 逻辑并集 {}x{} = {stage_scale:.4}，\
                 但显示器最大缩放是 {max_scale:.4}（{} 块屏）；几何按原样使用，覆盖层可能错位",
                desktop.width,
                desktop.height,
                monitors.len(),
            ));
            None
        }
    };

    let mut tiles = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let crop = scaled_monitor_rect(
            &monitor.rect,
            &desktop,
            scale_x,
            scale_y,
            image_width,
            image_height,
        )?;
        // 只有 `StageClass::Physical` 才修几何（逻辑尺寸不可信，按裁剪出的真实像素反推）。
        //
        // **除数是整台桌面的最大缩放，不是这块屏自己的缩放。** 这里的 `crop.width`
        // 来自整张舞台图，而舞台图的尺寸是"逻辑并集 × 各视图里最大的那个缩放"
        // （Mutter 的 `clutter_stage_get_capture_final_size`），低缩放的屏在图里是被
        // **放大**过的。拿这块屏自己的缩放去除，非最大缩放的那块屏就会被算出一个
        // 偏大的"逻辑尺寸"：实测 HDMI 2560x1440@1.5 + 笔记本 1920x1200@1.3333 时，
        // 笔记本被改写成 2160x1350@(2880,459)，偏差正好 1.5/1.3333 = 1.125 倍，
        // 于是覆盖层比屏幕大一圈、窗口候选整体左上偏移。单屏时自己的缩放就是最大缩放，
        // 所以这个错误一直藏着没露头。
        let mut adjusted = monitor.clone();
        if let Some(divisor) = repair_divisor {
            adjusted.rect = normalize_monitor_geometry(adjusted.rect, divisor, crop.width);
        }
        adjusted.scale_factor = portal_frame_scale_factor(&adjusted, crop);

        // **不变量 I2b**：裁剪不能被图像边界静默钳小。用**修正前**的矩形来算——
        // 裁剪就是从它来的，拿修正后的算等于自己验自己。
        let clamped = verify_crop_not_clamped(
            monitor.rect.width,
            monitor.rect.height,
            crop,
            scale_x,
            scale_y,
        );
        if clamped > 0.0 {
            warnings.push(format!(
                "I2b 显示器 {} 的裁剪被舞台图边界钳掉 {clamped:.0} 像素：几何说 {}x{} × {scale_x:.4}，\
                 实际切到 {}x{}；已拔掉的屏或陈几何？",
                monitor.id, monitor.rect.width, monitor.rect.height, crop.width, crop.height,
            ));
        }

        // **不变量 I3**：帧/逻辑比值在两个方向上必须一致。
        //
        // **不要再把旋转屏写成嫌疑人。** 舞台图是合成器合出来的桌面，旋转已经烤进去了，
        // 正常竖屏的逻辑矩形和裁剪都是竖的，比值一致（见 `geometry_check::verify_frame_isotropy`）。
        // 这里响意味着裁剪的朝向和几何声称的朝向对不上——几何是热插拔/改分辨率之前的陈数据。
        let anisotropy = verify_frame_isotropy(adjusted.rect.width, adjusted.rect.height, crop);
        if anisotropy > 0.0 {
            warnings.push(format!(
                "I3 显示器 {} 的帧缩放两个方向不一致（差 {anisotropy:.4}）：逻辑 {}x{}，\
                 帧 {}x{}；裁剪朝向和几何对不上，几何是改分辨率/热插拔之前的陈数据？",
                adjusted.id, adjusted.rect.width, adjusted.rect.height, crop.width, crop.height,
            ));
        }

        tiles.push(StageTile {
            monitor: adjusted,
            crop,
            mirror_of: None,
        });
    }

    // **镜像屏先摘出去。** 两块屏共用同一个逻辑矩形是投影时的正常配置，裁剪当然相同；
    // 把它当 I2a 报错等于"一接投影仪就报几何错"。摘出去之后 I2a 只剩下真正无法解释的
    // 部分重叠（已拔掉的屏、热插拔前的陈几何），指向性才有意义。
    let all_crops: Vec<ImageRect> = tiles.iter().map(|tile| tile.crop).collect();
    for (mirror, source) in find_mirror_sources(&all_crops) {
        tiles[mirror].mirror_of = Some(tiles[source].monitor.id);
    }

    // **不变量 I2a**：各屏裁剪不得重叠。注意**不检查铺满**：显示器并集经常不是矩形，
    // 空出来的区域是正常的，覆盖率只是诊断报告里的一个数字，见 `geometry_check`。
    let crops: Vec<ImageRect> = tiles
        .iter()
        .filter(|tile| tile.mirror_of.is_none())
        .map(|tile| tile.crop)
        .collect();
    if let Err(reason) = verify_crops_do_not_overlap(&crops) {
        warnings.push(format!("I2a {reason}"));
    }
    // 覆盖率也只算非镜像的那些，否则镜像会把同一块面积数两遍，算出 200%。
    let coverage = crop_coverage_ratio(&crops, image_width, image_height);

    Ok(StageSplitPlan {
        desktop,
        stage,
        coverage,
        tiles,
        warnings,
    })
}

/// 一整张舞台图怎么切：几何结论加上不变量自检的结果。
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(super) struct StageSplitPlan {
    pub desktop: DesktopBounds,
    pub stage: StageClass,
    /// 裁剪覆盖了舞台图的多大比例。小于 1 是正常的（并集不是矩形），只是一个可看的数字。
    pub coverage: f32,
    pub tiles: Vec<StageTile>,
    /// 没通过的不变量，人话描述。空表示全过。**不变量失败不中断截图**：
    /// 画面本身仍然可用，退化只是几何可能不准，硬失败反而让用户什么都截不到。
    pub warnings: Vec<String>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub(super) struct StageTile {
    /// 修正后的显示器几何（逻辑像素）与按帧算出的缩放。
    pub monitor: MonitorInfo,
    /// 这块屏在舞台图里的位置。
    pub crop: ImageRect,
    /// 裁剪和哪块屏**完全相同**（镜像/投影）。`None` 表示这块屏有自己的那份像素。
    /// 有值时切图会直接共享源屏那份缓冲，不再抠一遍。
    pub mirror_of: Option<u32>,
}

#[cfg(target_os = "linux")]
impl StageSplitPlan {
    /// 一行几何摘要，给日志和诊断报告共用。
    ///
    /// **不放窗口标题、不放像素**——这一行会跟着报障走，而标题会泄露用户在做什么
    /// （和扩展 `GetWindows` 的令牌是同一套威胁模型）。这里只有显示器几何和倍率。
    pub fn summary_line(
        &self,
        monitor_count: usize,
        image_width: u32,
        image_height: u32,
    ) -> String {
        let class = match self.stage {
            StageClass::Logical { stage_scale } => format!("logical×{stage_scale:.4}"),
            StageClass::Physical => "physical".to_string(),
            StageClass::Unknown {
                stage_scale,
                max_scale,
            } => format!("unknown（图/并集={stage_scale:.4}，max(scale)={max_scale:.4}）"),
        };
        let tiles = self
            .tiles
            .iter()
            .map(|tile| {
                // 镜像标出来。它不再算 I2a 失败，但**必须看得见**：几何重复和几何算错
                // 是两个不同的结论，而这一行是排障的起点。
                let mirror = match tile.mirror_of {
                    Some(source) => format!("（镜像自 #{source}）"),
                    None => String::new(),
                };
                format!(
                    "#{}@{},{} {}x{}×{:.4}→{}x{}{mirror}",
                    tile.monitor.id,
                    tile.monitor.rect.x,
                    tile.monitor.rect.y,
                    tile.monitor.rect.width,
                    tile.monitor.rect.height,
                    tile.monitor.scale_factor,
                    tile.crop.width,
                    tile.crop.height,
                )
            })
            .collect::<Vec<_>>()
            .join("，");
        format!(
            "截图几何：{monitor_count} 块屏，舞台图 {image_width}x{image_height}，\
             逻辑并集 {}x{}@{},{}，{class}，覆盖 {:.1}%，自检 {}；{tiles}",
            self.desktop.width,
            self.desktop.height,
            self.desktop.x,
            self.desktop.y,
            self.coverage * 100.0,
            if self.warnings.is_empty() {
                "通过".to_string()
            } else {
                format!("{} 条未通过", self.warnings.len())
            },
        )
    }
}

#[cfg(target_os = "linux")]
pub(super) fn split_portal_screenshot(
    monitors: Vec<MonitorInfo>,
    image: RgbaImage,
) -> Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)> {
    let image_width = image.width();
    let image_height = image.height();
    let plan = plan_stage_split(&monitors, image_width, image_height)?;
    // 每次截图都留一行几何摘要。**这行是排障的起点**：报障时用户看不见几何，
    // 而"覆盖层错位/画面溢到隔壁屏"这类症状的根因几乎都能从这一行读出来。
    log::info!(
        "{}",
        plan.summary_line(monitors.len(), image_width, image_height)
    );
    for warning in &plan.warnings {
        log::error!("截图几何自检未通过：{warning}");
    }

    let rgba = image.into_raw();
    let mut adjusted_monitors = Vec::with_capacity(plan.tiles.len());
    let mut frames = Vec::with_capacity(plan.tiles.len());

    for tile in plan.tiles {
        // 镜像屏的裁剪和源屏一模一样，抠出来的字节也一模一样。共享那份 `Arc` 省掉的是
        // 一整块屏的拷贝加一份同样大小的内存（1080p 就是 8 MB），而截图这条路上
        // 每一毫秒用户都在等。找不到源屏（理论上不会）时退回自己抠一份，绝不让截图失败。
        let shared = tile.mirror_of.and_then(|source| {
            frames
                .iter()
                .find(|frame: &&FrozenFrame| frame.monitor_id == source)
                .map(|frame| Arc::clone(&frame.rgba))
        });
        let rgba = match shared {
            Some(shared) => shared,
            None => Arc::from(crop_rgba(&rgba, image_width, tile.crop)?),
        };
        frames.push(FrozenFrame {
            monitor_id: tile.monitor.id,
            rgba,
            width: tile.crop.width,
            height: tile.crop.height,
            scale_factor: tile.monitor.scale_factor,
        });
        adjusted_monitors.push(tile.monitor);
    }

    Ok((adjusted_monitors, frames))
}

pub(super) fn monitor_union(monitors: &[MonitorInfo]) -> Result<DesktopBounds> {
    let min_x = monitors
        .iter()
        .map(|monitor| monitor.rect.x)
        .min()
        .context("缺少显示器 x 坐标")?;
    let min_y = monitors
        .iter()
        .map(|monitor| monitor.rect.y)
        .min()
        .context("缺少显示器 y 坐标")?;
    let max_x = monitors
        .iter()
        .map(|monitor| monitor.rect.x as i64 + monitor.rect.width as i64)
        .max()
        .context("缺少显示器右边界")?;
    let max_y = monitors
        .iter()
        .map(|monitor| monitor.rect.y as i64 + monitor.rect.height as i64)
        .max()
        .context("缺少显示器下边界")?;

    let width = max_x - min_x as i64;
    let height = max_y - min_y as i64;
    if width <= 0 || height <= 0 {
        bail!("显示器组合区域为空");
    }

    Ok(DesktopBounds {
        x: min_x,
        y: min_y,
        width: width as u32,
        height: height as u32,
    })
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn scaled_monitor_rect(
    rect: &Rect,
    desktop: &DesktopBounds,
    scale_x: f32,
    scale_y: f32,
    image_width: u32,
    image_height: u32,
) -> Result<ImageRect> {
    let left = scaled_edge(rect.x, desktop.x, scale_x, image_width);
    let top = scaled_edge(rect.y, desktop.y, scale_y, image_height);
    let right = scaled_edge(
        rect.x.saturating_add_unsigned(rect.width),
        desktop.x,
        scale_x,
        image_width,
    );
    let bottom = scaled_edge(
        rect.y.saturating_add_unsigned(rect.height),
        desktop.y,
        scale_y,
        image_height,
    );

    if right <= left || bottom <= top {
        bail!("缩放后的显示器区域为空");
    }

    Ok(ImageRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(any(test, target_os = "linux"))]
fn scaled_edge(edge: i32, origin: i32, scale: f32, max: u32) -> u32 {
    (((edge - origin) as f32) * scale)
        .round()
        .clamp(0.0, max as f32) as u32
}

#[cfg(target_os = "linux")]
fn crop_rgba(rgba: &[u8], image_width: u32, crop: ImageRect) -> Result<Vec<u8>> {
    let row_bytes = crop.width as usize * 4;
    let mut out = Vec::with_capacity(row_bytes * crop.height as usize);
    for row in 0..crop.height {
        let y = crop.y + row;
        let start = (y * image_width + crop.x) as usize * 4;
        let end = start + row_bytes;
        let slice = rgba
            .get(start..end)
            .with_context(|| format!("Portal 截图裁剪第 {row} 行越界"))?;
        out.extend_from_slice(slice);
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn portal_frame_scale_factor(monitor: &MonitorInfo, crop: ImageRect) -> f32 {
    let width_scale = if monitor.rect.width > 0 {
        crop.width as f32 / monitor.rect.width as f32
    } else {
        0.0
    };
    let height_scale = if monitor.rect.height > 0 {
        crop.height as f32 / monitor.rect.height as f32
    } else {
        0.0
    };
    let scale = width_scale.max(height_scale);

    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        monitor.scale_factor
    }
}

fn monitor_info(mon: &Monitor) -> Result<MonitorInfo> {
    let id = mon.id().context("无法读取显示器 id")?;
    let scale = mon.scale_factor().context("无法读取显示器缩放")?;
    let x = mon.x().context("无法读取显示器 x 坐标")?;
    let y = mon.y().context("无法读取显示器 y 坐标")?;
    let width = mon.width().context("无法读取显示器宽度")?;
    let height = mon.height().context("无法读取显示器高度")?;

    Ok(MonitorInfo {
        id,
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        scale_factor: scale,
    })
}

#[cfg(target_os = "linux")]
fn monitor_info_from_wayland_output(
    index: usize,
    output: &libwayshot_xcap::output::OutputInfo,
) -> MonitorInfo {
    let region = output.logical_region.inner;

    MonitorInfo {
        id: stable_wayland_output_id(index, output),
        rect: Rect {
            x: region.position.x,
            y: region.position.y,
            width: region.size.width,
            height: region.size.height,
        },
        scale_factor: wayland_output_scale_factor(output),
    }
}

#[cfg(target_os = "linux")]
fn stable_wayland_output_id(index: usize, output: &libwayshot_xcap::output::OutputInfo) -> u32 {
    let key = if output.name.is_empty() {
        output.description.as_str()
    } else {
        output.name.as_str()
    };
    let mut hash = 0x811c9dc5_u32;
    for byte in key
        .as_bytes()
        .iter()
        .copied()
        .chain([0xff])
        .chain(index.to_le_bytes())
    {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// 把逐输出抓来的像素转到**桌面朝向**。
///
/// 变换方向照抄 libwayshot 自己的 `image_util::rotate_image_buffer`（`Transform::_90`
/// 对应 `rotate90`，Flipped\* 先水平翻再转）：那是同一批缓冲的上游参考实现，
/// 方向搞反了画面会倒着或者镜像，而这种错在横屏上永远看不出来。
///
/// 转完宽高就和逻辑矩形同向了，I3 因此对正常竖屏不响。
#[cfg(target_os = "linux")]
pub(super) fn apply_output_transform(
    image: RgbaImage,
    transform: libwayshot_xcap::reexport::Transform,
) -> RgbaImage {
    use image::imageops::{flip_horizontal, rotate180, rotate270, rotate90};
    use libwayshot_xcap::reexport::Transform;
    match transform {
        Transform::_90 => rotate90(&image),
        Transform::_180 => rotate180(&image),
        Transform::_270 => rotate270(&image),
        Transform::Flipped => flip_horizontal(&image),
        Transform::Flipped90 => rotate90(&flip_horizontal(&image)),
        Transform::Flipped180 => rotate180(&flip_horizontal(&image)),
        Transform::Flipped270 => rotate270(&flip_horizontal(&image)),
        // `Normal` 和将来新增的取值都按原样走：不认识的变换宁可不转，也别转错。
        _ => image,
    }
}

/// 这个 `transform` 是否把面板的宽高换了个方向。
///
/// `wl_output::Transform` 的八个取值里，`_90` / `_270` 与它们的镜像版本是旋转 90 度的，
/// `Normal` / `_180` / `Flipped` / `Flipped180` 保持横竖不变。镜像（Flipped\*）只翻不转，
/// 对宽高没有影响，所以这里只看角度。
///
/// 类型走 `libwayshot_xcap::reexport`，不直接依赖 `wayland-client`：两边版本一旦错开，
/// 同名类型就不是同一个类型，编译错误会指向一个完全无关的地方。
#[cfg(target_os = "linux")]
pub(super) fn transform_swaps_axes(transform: libwayshot_xcap::reexport::Transform) -> bool {
    use libwayshot_xcap::reexport::Transform;
    matches!(
        transform,
        Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
    )
}

#[cfg(target_os = "linux")]
fn wayland_output_scale_factor(output: &libwayshot_xcap::output::OutputInfo) -> f32 {
    let logical = output.logical_region.inner.size;
    let physical = output.physical_size;
    // 竖屏必须换轴，否则算出的是 1.7778 这种彻底错的"缩放"，详见 `output_scale_from_sizes`。
    super::geometry_check::output_scale_from_sizes(
        (physical.width, physical.height),
        (logical.width, logical.height),
        transform_swaps_axes(output.transform),
    )
    .unwrap_or(1.0)
}

#[cfg(target_os = "linux")]
fn wayland_connection() -> Result<libwayshot_xcap::WayshotConnection> {
    std::panic::catch_unwind(libwayshot_xcap::WayshotConnection::new)
        .map_err(|_| anyhow!("Wayland 输出发现 panic"))?
        .context("无法连接 Wayland compositor")
}

#[cfg(target_os = "linux")]
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|session| session.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(any(test, target_os = "linux"))]
pub(super) fn portal_screenshot_uri_to_path(uri: &str) -> Result<std::path::PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or_else(|| anyhow!("Portal 截图 URI 不是本地 file URI: {uri}"))?;

    let path = if let Some(path) = rest.strip_prefix('/') {
        format!("/{path}")
    } else if let Some(path) = rest.strip_prefix("localhost/") {
        format!("/{path}")
    } else {
        let authority = rest.split('/').next().unwrap_or(rest);
        bail!("Portal 截图 URI authority 不支持 `{authority}`: {uri}");
    };

    let decoded = urlencoding::decode(&path)
        .with_context(|| format!("Portal 截图 URI path 不是合法 UTF-8: {uri}"))?;
    Ok(std::path::PathBuf::from(decoded.into_owned()))
}

#[cfg(target_os = "linux")]
fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{millis}")
}

/// 真机截图后端诊断。默认 `#[ignore]`，要有真实桌面会话才有意义：
/// `cargo test --lib backend_diagnostics -- --ignored --nocapture`
///
/// "截图是黑的"这类问题在单元测试里看不出来——链路每一环都返回 Ok，只是像素全 0。
/// 所以这里按 fallback 顺序逐个后端跑一遍，打印尺寸与平均亮度/全黑像素比例，
/// 一眼就能看出是哪个后端在给黑帧、以及它前面的后端为什么被跳过。
#[cfg(all(test, target_os = "linux"))]
mod backend_diagnostics {
    use super::*;

    fn describe(label: &str, result: Result<(Vec<MonitorInfo>, Vec<FrozenFrame>)>) {
        match result {
            Ok((infos, frames)) => {
                println!("[{label}] ok, {} 个显示器", frames.len());
                for (info, frame) in infos.iter().zip(frames.iter()) {
                    let pixels = frame.rgba.len() / 4;
                    let mut sum: u64 = 0;
                    let mut opaque_black = 0usize;
                    let mut transparent = 0usize;
                    for chunk in frame.rgba.as_chunks::<4>().0 {
                        let luma = chunk[0] as u64 + chunk[1] as u64 + chunk[2] as u64;
                        sum += luma;
                        if chunk[3] == 0 {
                            transparent += 1;
                        } else if luma == 0 {
                            opaque_black += 1;
                        }
                    }
                    let mean = if pixels == 0 {
                        0.0
                    } else {
                        sum as f64 / (pixels as f64 * 3.0)
                    };
                    println!(
                        "  monitor {} pos=({},{}) logical={}x{} frame={}x{} scale={:.2} 平均亮度={:.1} 全黑={:.1}% 全透明={:.1}%",
                        info.id,
                        info.rect.x,
                        info.rect.y,
                        info.rect.width,
                        info.rect.height,
                        frame.width,
                        frame.height,
                        frame.scale_factor,
                        mean,
                        100.0 * opaque_black as f64 / pixels.max(1) as f64,
                        100.0 * transparent as f64 / pixels.max(1) as f64,
                    );
                }
            }
            Err(error) => println!("[{label}] 失败: {error:#}"),
        }
    }

    #[test]
    #[ignore = "需要真实桌面会话"]
    fn backend_diagnostics() {
        println!(
            "session: XDG_SESSION_TYPE={:?} WAYLAND_DISPLAY={:?} DISPLAY={:?} is_wayland={}",
            std::env::var("XDG_SESSION_TYPE").ok(),
            std::env::var("WAYLAND_DISPLAY").ok(),
            std::env::var("DISPLAY").ok(),
            is_wayland_session(),
        );
        describe(
            "clippy shell extension（逐屏原生）",
            capture_all_shell_extension_monitor_areas(),
        );
        describe(
            "clippy shell extension（整屏舞台图）",
            capture_all_shell_extension_monitors(),
        );
        describe("wlroots/libwayshot", capture_all_wayland_monitors());
        describe("portal(non-interactive)", capture_all_portal_monitors());
        describe("gnome-shell", capture_all_gnome_shell_monitors());
        describe("xcap", capture_all_xcap_monitors());
        describe("实际选用的链路", capture_all_monitors());
    }

    /// 窗口枚举对截图链路的副作用诊断。报障是"加了窗口枚举之后截图变黑"，
    /// 而枚举本身在捕获之后才跑，所以要单独确认：枚举是否慢、是否会污染下一帧。
    /// 诊断只输出数量、状态与几何，不让窗口标题和进程标识离开当前进程。
    #[test]
    #[ignore = "需要真实桌面会话"]
    fn window_probe_diagnostics() {
        let before = std::time::Instant::now();
        let windows = xcap::Window::all();
        println!("Window::all() 耗时 {:?}", before.elapsed());
        match windows {
            Ok(list) => {
                println!("枚举到 {} 个窗口", list.len());
                for window in list.iter().take(12) {
                    println!(
                        "  minimized={:?} rect=({:?},{:?} {:?}x{:?})",
                        window.is_minimized(),
                        window.x(),
                        window.y(),
                        window.width(),
                        window.height(),
                    );
                }
            }
            Err(error) => println!("Window::all() 失败: {error}"),
        }
        describe("枚举窗口之后再截一次", capture_all_monitors());
    }
}
