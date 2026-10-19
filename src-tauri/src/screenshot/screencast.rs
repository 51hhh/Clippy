//! GNOME Wayland 上的首选取像素路径：`org.gnome.Mutter.ScreenCast` + PipeWire。
//!
//! ## 为什么是它（三个量过的数字）
//!
//! 1. **旧路径慢在 PNG，不慢在像素。** 同一批像素、两种处理的对照实验（拍一块屏，把 PNG
//!    解开，再用同一个 gdk-pixbuf 把**一模一样的像素**重编一次）：4K 那块屏端到端 1704 ms，
//!    本地重编 1607 ms —— **94% 的时间是 deflate**，合成器绘制 + 读回只有约 100 ms。
//!    而 `Shell.Screenshot` 不暴露压缩档位，那 1.6 秒没有参数可调。
//! 2. **"在扩展里绕开 PNG、直接读原始像素"在 GJS 上不可能成立。** `Cogl.Texture.get_data`
//!    的 `data` 参数是**没有长度标注**的 `array<uint8>`，GJS 会把 `Uint8Array` 复制一份
//!    传给 C、调用结束就释放（同形状的 `GdkPixbuf.Pixbuf.new_from_data` 别名实验：4×4
//!    看不见改动、196608 字节读回来全是垃圾、3 MB 直接段错误）。那条路只能靠哨兵字节拦住
//!    "黑图"，永远快不了，见 `docs/capture-linux.md` §3.1。
//! 3. **Mutter 的 ScreenCast 同一个用户直接可调**：不经 Portal、不弹授权对话框、不需要
//!    restore token。逐屏 `RecordArea` 实测一个会话同时录两块屏 **173 ms**，拿到的是
//!    3840x2160 与 2560x1600 的**原生像素**，不含鼠标指针。这里不能换成看起来更直接的
//!    `RecordMonitor`：GNOME 50.1 上外接屏的源会进入 streaming 却不送首帧，白等超时；
//!    同轮 A/B 中它耗时 378 ms 且缺一屏，`RecordArea` 两屏均成功。
//!
//! 也就是说分辨率和速度出自同一个改动，不是二选一。
//!
//! ## 代价，以及为什么必须传 `is-recording: true`
//!
//! 取流期间 GNOME 顶栏会闪一下 `media-record-symbolic` 隐私点（约 200 ms）。这是 GNOME
//! 对"有人在读屏幕"的诚实提示，不该也不能绕开。但 `is-recording` 传 false 会落到
//! `ScreenSharingIndicator` 那条分支上，而那个"停止共享"胶囊有 **5 秒最短显示时间**
//! （`js/ui/status/remoteAccess.js` 的 `MIN_SHARED_INDICATOR_VISIBLE_TIME_US`）——
//! 一次截图在顶栏留五秒的胶囊，比闪一下的小红点糟得多。
//!
//! ## 线程模型
//!
//! 整段（zbus 会话 + PipeWire main loop）跑在 `dbus::off_async_runtime` 借来的一条干净
//! OS 线程上，理由有两个：调用方可能已经在 tokio worker 线程上（在那里新建 runtime 会
//! `Cannot start a runtime from within a runtime`）；而 PipeWire 的 main loop 会把所在
//! 线程占住，不能占住 tokio 的 worker。
//!
//! 会话的生命周期绑在**创建它的那条 D-Bus 连接**上：Mutter 在对端断开时销毁会话，
//! 所以连接必须活到取完帧。`Stop` 由 RAII 守卫无条件发出，否则顶栏的录制点会一直亮着。

use crate::pipewire_frame::{
    enum_format_pod, frame_from_buffer, init_pipewire, parse_video_format,
};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::stream::StreamExt;
use pipewire as pw;
use pw::spa;
use spa::param::video::VideoInfoRaw;
use spa::pod::Pod;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, Value};

use super::Rect;

const SCREEN_CAST_NAME: &str = "org.gnome.Mutter.ScreenCast";
const SCREEN_CAST_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SESSION_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Session";
const STREAM_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";

/// `MetaCursorMode` 的 `HIDDEN`：冻结帧里不要鼠标指针（覆盖层自己画）。
const CURSOR_MODE_HIDDEN: u32 = 0;

/// 单次 D-Bus 往返的上限。实测 CreateSession / RecordArea 各 1 ms、Start 61 ms，
/// 留出两个数量级的余量：超时就退回下一条路，绝不能让截图卡在这里。
const CALL_TIMEOUT: Duration = Duration::from_millis(1500);

/// 从 `Start` 到收到第一帧的上限（含等 node id）。实测 18 ms + 104 ms。
// 正常首帧实测约 100 ms。某个输出若进入 running 却不送帧，继续等到 1.5 秒不会让它
// 恢复，只会把截图手感拖垮；350 ms 后把这一块交给逐屏原始像素兜底，其余已到帧照用。
const FRAME_TIMEOUT: Duration = Duration::from_millis(350);

/// 一块屏的原生像素。宽高是**实际拿到的帧**，不是我们要求的尺寸。
pub(super) struct ScreencastFrame {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba: Arc<[u8]>,
}

/// 逐屏取一帧原生像素，顺序与 `areas` 一致。
///
/// `RecordArea` 的坐标是 Mutter stage 的逻辑坐标；每次只传一块显示器的逻辑矩形，输出就会
/// 按该屏自己的 DPI 缩放到原生像素。字符串只用于日志，不参与 D-Bus 定位。
pub(super) fn capture_areas(areas: &[(String, Rect)]) -> Result<Vec<Result<ScreencastFrame>>> {
    if areas.is_empty() {
        bail!("没有要录制的显示器区域");
    }
    let sources = areas
        .iter()
        .map(|(label, rect)| ScreencastSource::Area {
            label: label.clone(),
            rect: *rect,
        })
        .collect::<Vec<_>>();
    crate::dbus::off_async_runtime(|| capture_off_runtime(&sources))
}

/// 只保留作 `RecordArea` / `RecordMonitor` 真机 A/B；生产路径必须走前者，见文件头。
#[cfg(test)]
fn capture_monitors(connectors: &[String]) -> Result<Vec<Result<ScreencastFrame>>> {
    if connectors.is_empty() {
        bail!("没有要录制的显示器");
    }
    let sources = connectors
        .iter()
        .map(|connector| ScreencastSource::Monitor {
            connector: connector.clone(),
        })
        .collect::<Vec<_>>();
    crate::dbus::off_async_runtime(|| capture_off_runtime(&sources))
}

#[derive(Clone)]
enum ScreencastSource {
    #[cfg(test)]
    Monitor {
        connector: String,
    },
    Area {
        label: String,
        rect: Rect,
    },
}

impl ScreencastSource {
    fn label(&self) -> &str {
        match self {
            #[cfg(test)]
            Self::Monitor { connector } => connector,
            Self::Area { label, .. } => label,
        }
    }
}

fn capture_off_runtime(sources: &[ScreencastSource]) -> Result<Vec<Result<ScreencastFrame>>> {
    // current_thread：这条线程接下来要被 PipeWire 的 main loop 占住，多开工作线程没意义。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法创建 ScreenCast runtime")?;
    let session = runtime.block_on(open_session(sources))?;
    let session = SessionGuard {
        runtime: &runtime,
        session,
    };
    let frames = pull_first_frames(&session.session.nodes)?;
    Ok(frames)
}

/// 一个活着的录制会话。`_connection` 只为"别断开"而存在——Mutter 在创建会话的那条连接
/// 断开时立刻销毁会话，连 `Stop` 都来不及发。
struct Session {
    _connection: zbus::Connection,
    session: zbus::Proxy<'static>,
    nodes: Vec<StreamNode>,
}

struct StreamNode {
    connector: String,
    node_id: u32,
}

/// 无论成功失败都把 `Stop` 发出去：不发的话顶栏的录制点会一直亮到进程退出。
struct SessionGuard<'r> {
    runtime: &'r tokio::runtime::Runtime,
    session: Session,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        let stop = with_timeout("Stop", CALL_TIMEOUT, async {
            self.session
                .session
                .call::<_, _, ()>("Stop", &())
                .await
                .context("Stop 调用失败")
        });
        if let Err(error) = self.runtime.block_on(stop) {
            log::warn!("关闭 Mutter 录制会话失败（顶栏的录制点可能要等到进程退出）：{error:#}");
        }
    }
}

async fn open_session(sources: &[ScreencastSource]) -> Result<Session> {
    let connection = with_timeout("连接 session bus", CALL_TIMEOUT, async {
        zbus::Connection::session()
            .await
            .context("无法连接 session bus")
    })
    .await?;

    let screen_cast = proxy(&connection, SCREEN_CAST_PATH, SCREEN_CAST_NAME).await?;
    let options: HashMap<&str, Value> = HashMap::new();
    let session_path: OwnedObjectPath = with_timeout("CreateSession", CALL_TIMEOUT, async {
        screen_cast
            .call("CreateSession", &(options,))
            .await
            .context("CreateSession 调用失败（不是 GNOME？）")
    })
    .await?;
    let session = proxy(&connection, session_path, SESSION_INTERFACE).await?;

    let mut stream_paths = Vec::with_capacity(sources.len());
    for source in sources {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("cursor-mode", Value::U32(CURSOR_MODE_HIDDEN));
        // 见文件头：false 会换来一个 5 秒起步的"停止共享"胶囊。
        options.insert("is-recording", Value::Bool(true));
        let label = source.label().to_string();
        let path: OwnedObjectPath = match source {
            #[cfg(test)]
            ScreencastSource::Monitor { connector } => {
                with_timeout("RecordMonitor", CALL_TIMEOUT, async {
                    session
                        .call("RecordMonitor", &(connector.as_str(), options))
                        .await
                        .with_context(|| format!("RecordMonitor {connector} 失败"))
                })
                .await?
            }
            ScreencastSource::Area { rect, .. } => {
                let width = i32::try_from(rect.width).context("RecordArea 宽度超过 i32")?;
                let height = i32::try_from(rect.height).context("RecordArea 高度超过 i32")?;
                with_timeout("RecordArea", CALL_TIMEOUT, async {
                    session
                        .call("RecordArea", &(rect.x, rect.y, width, height, options))
                        .await
                        .with_context(|| format!("RecordArea {label} 失败"))
                })
                .await?
            }
        };
        stream_paths.push((label, path));
    }

    // **订阅必须早于 `Start`。** node id 只从 `PipeWireStreamAdded` 来，而它在 `Start`
    // 之后 18 ms 就到了；先 Start 再订阅就是在赌信号还没发出去。
    let mut pending = Vec::with_capacity(stream_paths.len());
    for (connector, path) in stream_paths {
        let stream = proxy(&connection, path, STREAM_INTERFACE).await?;
        let signals = stream
            .receive_signal("PipeWireStreamAdded")
            .await
            .with_context(|| format!("无法订阅 {connector} 的 PipeWireStreamAdded"))?;
        pending.push((connector, signals));
    }

    with_timeout("Start", CALL_TIMEOUT, async {
        session
            .call::<_, _, ()>("Start", &())
            .await
            .context("Start 调用失败")
    })
    .await?;

    let mut nodes = Vec::with_capacity(pending.len());
    for (connector, mut signals) in pending {
        let message = with_timeout("PipeWireStreamAdded", FRAME_TIMEOUT, async {
            signals
                .next()
                .await
                .ok_or_else(|| anyhow!("{connector} 的信号流提前结束"))
        })
        .await?;
        let (node_id,): (u32,) = message
            .body()
            .deserialize()
            .context("PipeWireStreamAdded 的载荷不是 (u)")?;
        nodes.push(StreamNode { connector, node_id });
    }

    Ok(Session {
        _connection: connection,
        session,
        nodes,
    })
}

/// 建一个不缓存属性的代理。属性一个都不读，缓存只会白搭一次 `GetAll` 往返。
async fn proxy<'p, P>(
    connection: &zbus::Connection,
    path: P,
    interface: &'static str,
) -> Result<zbus::Proxy<'p>>
where
    P: TryInto<zbus::zvariant::ObjectPath<'p>>,
    P::Error: Into<zbus::Error>,
{
    zbus::proxy::Builder::new(connection)
        .destination(SCREEN_CAST_NAME)
        .and_then(|builder| builder.path(path))
        .and_then(|builder| builder.interface(interface))
        .map_err(anyhow::Error::from)
        .context("无法构造 ScreenCast 代理")?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .context("无法创建 ScreenCast 代理")
}

/// 每一次等待都要有上限：截图这条路上宁可退回慢的后端，也不能挂住不动。
async fn with_timeout<T>(
    what: &str,
    limit: Duration,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(limit, work).await {
        Ok(result) => result,
        Err(_) => bail!("{what} 超过 {} ms 未返回", limit.as_millis()),
    }
}

/// 每块屏取第一帧。所有 node 共用一个 main loop，帧之间天然并行。
fn pull_first_frames(nodes: &[StreamNode]) -> Result<Vec<Result<ScreencastFrame>>> {
    init_pipewire();
    let mainloop = pw::main_loop::MainLoopRc::new(None).context("无法创建 PipeWire main loop")?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).context("无法创建 PipeWire 上下文")?;
    let core = context.connect_rc(None).context("无法连上 PipeWire")?;

    let slots = Rc::new(RefCell::new(
        nodes.iter().map(|_| Slot::default()).collect::<Vec<_>>(),
    ));
    let remaining = Rc::new(std::cell::Cell::new(nodes.len()));
    let format = enum_format_pod()?;

    // **声明顺序决定析构顺序**：listener 必须先于 stream 析构（它的 Drop 要把 hook 从
    // stream 的链表里摘掉，stream 先被 destroy 掉就是 use-after-free）。局部变量按声明
    // 的**逆序**析构，所以 streams 写在前面。
    let mut streams = Vec::with_capacity(nodes.len());
    let mut listeners = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let stream = pw::stream::StreamRc::new(
            core.clone(),
            "clippy-frozen-frame",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )
        .context("无法创建 PipeWire 流")?;

        let listener = stream
            .add_local_listener_with_user_data(StreamUserData {
                index,
                connector: node.connector.clone(),
                slots: Rc::clone(&slots),
                remaining: Rc::clone(&remaining),
                mainloop: mainloop.clone(),
                format: None,
                done: false,
            })
            .state_changed(|_, data, _, new| {
                if let pw::stream::StreamState::Error(message) = new {
                    data.finish(Err(anyhow!("PipeWire 流报错：{message}")));
                }
            })
            .param_changed(|_, data, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                match parse_video_format(param) {
                    Ok(info) => data.format = Some(info),
                    Err(error) => data.finish(Err(error)),
                }
            })
            .process(|stream, data| {
                if data.done {
                    return;
                }
                let Some(info) = data.format else {
                    return;
                };
                // 只取第一帧就退出，所以不必排空队列：这条路的调用方是"按下快捷键的
                // 那一刻"，多等一帧只是让画面更旧。
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let result = frame_from_buffer(&mut buffer, info).map(|frame| ScreencastFrame {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba,
                });
                data.finish(result);
            })
            .register()
            .context("无法注册 PipeWire 流回调")?;

        let mut params = [Pod::from_bytes(&format).context("EnumFormat pod 不合法")?];
        stream
            .connect(
                spa::utils::Direction::Input,
                Some(node.node_id),
                // 一次性截图不能依赖源节点恰好自行调度。PipeWire 1.x 上外接 4K 输出可能
                // 已进入 streaming 却保持 driving=0，结果永远没有首帧；DRIVER 让这个
                // 输入流主动驱动图，收到第一帧后仍会立即退出。
                pw::stream::StreamFlags::AUTOCONNECT
                    | pw::stream::StreamFlags::MAP_BUFFERS
                    | pw::stream::StreamFlags::DRIVER,
                &mut params,
            )
            .with_context(|| {
                format!(
                    "无法连上 {} 的 PipeWire node {}",
                    node.connector, node.node_id
                )
            })?;

        streams.push(stream);
        listeners.push(listener);
    }

    // DRIVER 流必须由消费者主动触发；PipeWire 也明确说明一次 graph iteration 可能没有
    // 完成，所以不能只触发一次。16 ms 的短周期只活到首帧到达（常态不足 200 ms），
    // 对未成为 driver 的流则只是向已有 driver 发送 RequestProcess。
    let trigger_streams: Vec<_> = streams.iter().map(|stream| stream.downgrade()).collect();
    let trigger = mainloop.loop_().add_timer(move |_| {
        for stream in &trigger_streams {
            if let Some(stream) = stream.upgrade() {
                let _ = stream.trigger_process();
            }
        }
    });
    trigger
        .update_timer(
            Some(Duration::from_millis(1)),
            Some(Duration::from_millis(16)),
        )
        .into_result()
        .map_err(|error| anyhow!("无法启动 PipeWire 首帧驱动器：{error}"))?;

    // 看门狗：一帧都没来也要退出，否则 main loop 会一直转下去。
    let watchdog = mainloop.loop_().add_timer({
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });
    watchdog
        .update_timer(Some(FRAME_TIMEOUT), None)
        .into_result()
        .map_err(|error| anyhow!("无法给 PipeWire 看门狗上弦：{error}"))?;
    mainloop.run();

    drop(trigger);
    drop(listeners);
    drop(streams);

    let mut frames = Vec::with_capacity(nodes.len());
    for (node, slot) in nodes.iter().zip(slots.borrow_mut().iter_mut()) {
        match slot.frame.take() {
            Some(Ok(frame)) => frames.push(Ok(frame)),
            Some(Err(error)) => frames.push(Err(
                error.context(format!("显示器 {} 取流失败", node.connector))
            )),
            None => frames.push(Err(anyhow!(
                "显示器 {} 在 {} ms 内没有送出画面",
                node.connector,
                FRAME_TIMEOUT.as_millis()
            ))),
        }
    }
    Ok(frames)
}

#[derive(Default)]
struct Slot {
    frame: Option<Result<ScreencastFrame>>,
}

struct StreamUserData {
    index: usize,
    connector: String,
    slots: Rc<RefCell<Vec<Slot>>>,
    remaining: Rc<std::cell::Cell<usize>>,
    mainloop: pw::main_loop::MainLoopRc,
    format: Option<VideoInfoRaw>,
    done: bool,
}

impl StreamUserData {
    /// 记下这块屏的结果；全部有结果就退出 main loop。
    fn finish(&mut self, result: Result<ScreencastFrame>) {
        if self.done {
            return;
        }
        self.done = true;
        if let Err(error) = &result {
            log::warn!("显示器 {} 的 PipeWire 流失败：{error:#}", self.connector);
        }
        if let Ok(mut slots) = self.slots.try_borrow_mut() {
            slots[self.index].frame = Some(result);
        }
        let left = self.remaining.get().saturating_sub(1);
        self.remaining.set(left);
        if left == 0 {
            self.mainloop.quit();
        }
    }
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
