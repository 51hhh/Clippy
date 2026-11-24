//! GNOME Wayland 的逐屏冻结帧使用 Mutter ScreenCast 与 PipeWire。
//!
//! 调用者在捕获线程执行，D-Bus 连接的生命周期覆盖首帧与 Stop。
use super::pipewire_frame::{
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
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, Value};

use super::{DesktopRect as Rect, FrozenPixels as ScreencastFrame};

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

/// 逐屏取一帧原生像素，顺序与 `areas` 一致。
///
/// `RecordArea` 的坐标是 Mutter stage 的逻辑坐标；每次只传一块显示器的逻辑矩形，输出就会
/// 按该屏自己的 DPI 缩放到原生像素。字符串只用于日志，不参与 D-Bus 定位。
pub fn capture_areas(areas: &[(String, Rect)]) -> Result<Vec<Result<ScreencastFrame>>> {
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
    capture_off_runtime(&sources)
}

/// 只保留作 `RecordArea` / `RecordMonitor` 真机 A/B；生产路径必须走前者，见文件头。
#[doc(hidden)]
pub fn capture_monitors(connectors: &[String]) -> Result<Vec<Result<ScreencastFrame>>> {
    if connectors.is_empty() {
        bail!("没有要录制的显示器");
    }
    let sources = connectors
        .iter()
        .map(|connector| ScreencastSource::Monitor {
            connector: connector.clone(),
        })
        .collect::<Vec<_>>();
    capture_off_runtime(&sources)
}

#[derive(Clone)]
enum ScreencastSource {
    Monitor { connector: String },
    Area { label: String, rect: Rect },
}

impl ScreencastSource {
    fn label(&self) -> &str {
        match self {
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
type FrameBatch = Vec<Result<ScreencastFrame>>;

fn pull_first_frames(nodes: &[StreamNode]) -> Result<FrameBatch> {
    with_frame_reader(nodes, |read| read())
}

pub(super) fn with_area<T>(
    name: String,
    rectangle: Rect,
    use_stream: impl FnOnce(&mut dyn FnMut() -> Result<ScreencastFrame>) -> Result<T>,
) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let session = SessionGuard {
        runtime: &runtime,
        session: runtime.block_on(open_session(&[ScreencastSource::Area {
            label: name,
            rect: rectangle,
        }]))?,
    };
    with_frame_reader(&session.session.nodes, |read| {
        use_stream(&mut || read()?.pop().expect("会话只订阅一个目标画面"))
    })
}

fn with_frame_reader<T>(
    nodes: &[StreamNode],
    use_stream: impl FnOnce(&mut dyn FnMut() -> Result<FrameBatch>) -> Result<T>,
) -> Result<T> {
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
                if data.completed() {
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
    let watchdog = mainloop.loop_().add_timer({
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });
    let result = use_stream(&mut || {
        remaining.set(nodes.len());
        trigger
            .update_timer(
                Some(Duration::from_millis(1)),
                Some(Duration::from_millis(16)),
            )
            .into_result()
            .map_err(|error| anyhow!("无法启动 PipeWire 首帧驱动器：{error}"))?;
        watchdog
            .update_timer(Some(FRAME_TIMEOUT), None)
            .into_result()
            .map_err(|error| anyhow!("无法给 PipeWire 看门狗上弦：{error}"))?;
        mainloop.run();
        let mut slots = slots.borrow_mut();
        Ok(nodes
            .iter()
            .zip(slots.iter_mut())
            .map(|(node, slot)| match slot.frame.take() {
                Some(Ok(frame)) => Ok(frame),
                Some(Err(error)) => {
                    Err(error.context(format!("显示器 {} 取流失败", node.connector)))
                }
                None => Err(anyhow!(
                    "显示器 {} 在 {} ms 内没有送出画面",
                    node.connector,
                    FRAME_TIMEOUT.as_millis()
                )),
            })
            .collect())
    });

    drop(trigger);
    drop(listeners);
    drop(streams);

    result
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
}

impl StreamUserData {
    fn completed(&self) -> bool {
        self.slots.borrow()[self.index].frame.is_some()
    }

    /// 记下这块屏的结果；全部有结果就退出 main loop。
    fn finish(&mut self, result: Result<ScreencastFrame>) {
        if self.completed() {
            return;
        }
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
