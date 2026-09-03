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
//!    restore token。本机实测 CreateSession 1 ms、RecordMonitor 1 ms、Start 61 ms、
//!    node 就绪 18 ms、第一帧 104 ms —— 单屏 **185 ms**；一个会话同时录两块屏 **190 ms**，
//!    拿到的是 3840x2160 与 2560x1600 的**原生像素**，不含鼠标指针。
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

use anyhow::{anyhow, bail, Context, Result};
use futures_util::stream::StreamExt;
use pipewire as pw;
use pw::spa;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::pod::Pod;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, Value};

const SCREEN_CAST_NAME: &str = "org.gnome.Mutter.ScreenCast";
const SCREEN_CAST_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SESSION_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Session";
const STREAM_INTERFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";

/// `MetaCursorMode` 的 `HIDDEN`：冻结帧里不要鼠标指针（覆盖层自己画）。
const CURSOR_MODE_HIDDEN: u32 = 0;

/// 单次 D-Bus 往返的上限。实测 CreateSession / RecordMonitor 各 1 ms、Start 61 ms，
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

/// 逐屏取一帧原生像素，顺序与 `connectors` 一致。
///
/// `connectors` 是 Wayland 输出名（`eDP-1`、`HDMI-1`……），也就是 `RecordMonitor`
/// 认的那个字符串。
pub(super) fn capture_monitors(connectors: &[String]) -> Result<Vec<Result<ScreencastFrame>>> {
    if connectors.is_empty() {
        bail!("没有要录制的显示器");
    }
    crate::dbus::off_async_runtime(|| capture_off_runtime(connectors))
}

fn capture_off_runtime(connectors: &[String]) -> Result<Vec<Result<ScreencastFrame>>> {
    // current_thread：这条线程接下来要被 PipeWire 的 main loop 占住，多开工作线程没意义。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("无法创建 ScreenCast runtime")?;
    let session = runtime.block_on(open_session(connectors))?;
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

async fn open_session(connectors: &[String]) -> Result<Session> {
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

    let mut stream_paths = Vec::with_capacity(connectors.len());
    for connector in connectors {
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("cursor-mode", Value::U32(CURSOR_MODE_HIDDEN));
        // 见文件头：false 会换来一个 5 秒起步的"停止共享"胶囊。
        options.insert("is-recording", Value::Bool(true));
        let path: OwnedObjectPath = with_timeout("RecordMonitor", CALL_TIMEOUT, async {
            session
                .call("RecordMonitor", &(connector.as_str(), options))
                .await
                .with_context(|| format!("RecordMonitor {connector} 失败"))
        })
        .await?;
        stream_paths.push((connector.clone(), path));
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
                let result = frame_from_buffer(&mut buffer, info);
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

fn init_pipewire() {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(pw::init);
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

fn parse_video_format(param: &Pod) -> Result<VideoInfoRaw> {
    let (media_type, media_subtype) = spa::param::format_utils::parse_format(param)
        .map_err(|error| anyhow!("无法解析 PipeWire 格式：{error}"))?;
    if media_type != MediaType::Video || media_subtype != MediaSubtype::Raw {
        bail!("PipeWire 协商出的不是原始视频（{media_type:?}/{media_subtype:?}）");
    }
    let mut info = VideoInfoRaw::new();
    info.parse(param)
        .map_err(|error| anyhow!("无法解析视频格式：{error}"))?;
    Ok(info)
}

fn frame_from_buffer(
    buffer: &mut pw::buffer::Buffer,
    info: VideoInfoRaw,
) -> Result<ScreencastFrame> {
    let size = info.size();
    let layout = pixel_layout(info.format()).ok_or_else(|| {
        anyhow!(
            "PipeWire 协商出的像素格式 {:?} 不是 32 位 RGB 排列",
            info.format()
        )
    })?;
    let datas = buffer.datas_mut();
    let data = datas.first_mut().context("PipeWire 缓冲里没有数据块")?;
    let kind = data.type_();
    if kind == spa::buffer::DataType::DmaBuf {
        // EnumFormat 里没有 `modifier` 属性，Mutter 就该走 shm/MemFd；真收到 DMA-BUF
        // 说明协商出了别的结果，而 MAP_BUFFERS 不会替我们 mmap 它。
        bail!("PipeWire 送来的是 DMA-BUF，这条路只处理共享内存");
    }
    let stride = data.chunk().stride();
    if stride <= 0 {
        bail!("PipeWire 帧的 stride 是 {stride}");
    }
    let offset = data.chunk().offset() as usize;
    let pixels = data.data().context("PipeWire 缓冲没有映射到内存")?;
    let pixels = pixels
        .get(offset..)
        .with_context(|| format!("PipeWire 帧的 offset {offset} 越过了缓冲末尾"))?;
    let rgba = repack_to_rgba(pixels, size.width, size.height, stride as usize, layout)?;
    Ok(ScreencastFrame {
        width: size.width,
        height: size.height,
        rgba: Arc::from(rgba),
    })
}

/// 4 字节像素里 R/G/B 各自的字节下标。SPA 的格式名说的是**内存里的字节序**
/// （`BGRx` 就是 B、G、R、填充），所以这里只是一张下标表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelLayout {
    r: usize,
    g: usize,
    b: usize,
}

/// 我们愿意接的 8 种 32 位排列，和 `enum_format_pod` 里报出去的那一组必须一致。
fn pixel_layout(format: VideoFormat) -> Option<PixelLayout> {
    const RGB: PixelLayout = PixelLayout { r: 0, g: 1, b: 2 };
    const BGR: PixelLayout = PixelLayout { r: 2, g: 1, b: 0 };
    const ARGB: PixelLayout = PixelLayout { r: 1, g: 2, b: 3 };
    const ABGR: PixelLayout = PixelLayout { r: 3, g: 2, b: 1 };

    if format == VideoFormat::RGBx || format == VideoFormat::RGBA {
        Some(RGB)
    } else if format == VideoFormat::BGRx || format == VideoFormat::BGRA {
        Some(BGR)
    } else if format == VideoFormat::xRGB || format == VideoFormat::ARGB {
        Some(ARGB)
    } else if format == VideoFormat::xBGR || format == VideoFormat::ABGR {
        Some(ABGR)
    } else {
        None
    }
}

/// 把一帧转成紧排的 RGBA8。
///
/// 去填充和换通道顺序一起做，因为都要逐行走一遍。**alpha 一律写 255**：桌面画面没有
/// 透明度，而 `BGRx` 这类格式里那个字节是未定义的，照抄会让整张图变成半透明。
fn repack_to_rgba(
    src: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    layout: PixelLayout,
) -> Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        bail!("PipeWire 帧的尺寸是 {width}x{height}");
    }
    let row = width.checked_mul(4).context("帧宽度溢出")?;
    if stride < row {
        bail!("PipeWire 帧的 stride {stride} 装不下一行 {row} 字节");
    }
    // 最后一行不需要行尾填充，所以下限是 stride × (行数 − 1) + 一行的有效字节。
    let minimum = stride
        .checked_mul(height - 1)
        .and_then(|value| value.checked_add(row))
        .context("帧尺寸溢出")?;
    if src.len() < minimum {
        bail!(
            "PipeWire 帧只有 {} 字节，装不下 {width}x{height}（stride {stride}）",
            src.len()
        );
    }

    let mut out = vec![0u8; row * height];
    for (y, line) in out.chunks_exact_mut(row).enumerate() {
        let source = &src[y * stride..y * stride + row];
        // row 是 4 的整数倍，两边的余数段都是空的。
        let (targets, _) = line.as_chunks_mut::<4>();
        let (pixels, _) = source.as_chunks::<4>();
        for (target, pixel) in targets.iter_mut().zip(pixels) {
            target[0] = pixel[layout.r];
            target[1] = pixel[layout.g];
            target[2] = pixel[layout.b];
            target[3] = 0xff;
        }
    }
    Ok(out)
}

/// 报给 Mutter 的 `EnumFormat`。
///
/// **故意不带 `modifier` 属性**：带了 Mutter 就会尝试 DMA-BUF，而我们要的是能直接
/// memcpy 的共享内存。尺寸与帧率给的是宽范围——真正的尺寸由显示器决定，写死只会让协商失败。
fn enum_format_pod() -> Result<Vec<u8>> {
    use spa::pod::{object, property, Value};
    use spa::utils::{Fraction, Rectangle};

    let object = object! {
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        property!(spa::param::format::FormatProperties::MediaType, Id, MediaType::Video),
        property!(spa::param::format::FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::RGBx,
            VideoFormat::BGRA,
            VideoFormat::RGBA,
            VideoFormat::xRGB,
            VideoFormat::xBGR,
            VideoFormat::ARGB,
            VideoFormat::ABGR,
        ),
        property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle { width: 1920, height: 1080 },
            Rectangle { width: 1, height: 1 },
            Rectangle { width: 16384, height: 16384 }
        ),
        property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 60, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: 1000, denom: 1 }
        ),
    };

    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .map_err(|error| anyhow!("无法序列化 EnumFormat：{error}"))?
    .0
    .into_inner();
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8 种排列都要认得，而且和 `enum_format_pod` 报出去的那一组一一对应——
    /// 报了却不认，协商成功之后才在运行时失败，那时候已经退不回别的后端了。
    #[test]
    fn every_advertised_format_has_a_layout() {
        for format in [
            VideoFormat::RGBx,
            VideoFormat::RGBA,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::xRGB,
            VideoFormat::ARGB,
            VideoFormat::xBGR,
            VideoFormat::ABGR,
        ] {
            assert!(
                pixel_layout(format).is_some(),
                "报给 Mutter 的格式 {format:?} 没有对应的通道下标"
            );
        }
        assert!(pixel_layout(VideoFormat::NV12).is_none());
        assert!(pixel_layout(VideoFormat::RGB).is_none());
    }

    /// Mutter 在小端上通常给 `BGRx`：字节序是 B、G、R、填充，换成 RGBA 要交换首尾。
    #[test]
    fn bgrx_becomes_rgba_with_opaque_alpha() {
        let layout = pixel_layout(VideoFormat::BGRx).expect("BGRx 必须认");
        // 一个像素：B=1 G=2 R=3，填充字节故意写成垃圾。
        let frame = repack_to_rgba(&[1, 2, 3, 0x7f], 1, 1, 4, layout).expect("转换失败");
        assert_eq!(frame, vec![3, 2, 1, 0xff]);
    }

    /// 行尾填充必须被丢掉，否则整张图从第二行开始就斜了。
    #[test]
    fn row_padding_is_dropped() {
        let layout = pixel_layout(VideoFormat::RGBx).expect("RGBx 必须认");
        let mut src = Vec::new();
        for row in 0..2u8 {
            src.extend_from_slice(&[row, row, row, 0]);
            src.extend_from_slice(&[0xee; 8]); // 填充，不该出现在结果里
        }
        let frame = repack_to_rgba(&src, 1, 2, 12, layout).expect("转换失败");
        assert_eq!(frame, vec![0, 0, 0, 0xff, 1, 1, 1, 0xff]);
    }

    /// 最后一行没有填充：下限按 `stride × (行数−1) + 一行` 算，按 `stride × 行数`
    /// 算会把合法的帧判成截断。
    #[test]
    fn the_last_row_needs_no_padding() {
        let layout = pixel_layout(VideoFormat::RGBx).expect("RGBx 必须认");
        let src = vec![0u8; 12 + 4];
        assert!(repack_to_rgba(&src, 1, 2, 12, layout).is_ok());
        assert!(repack_to_rgba(&src[..15], 1, 2, 12, layout).is_err());
    }

    /// 越界的形状一律要报错而不是 panic：这些数字来自合成器，不是我们算出来的。
    #[test]
    fn impossible_shapes_are_rejected() {
        let layout = PixelLayout { r: 0, g: 1, b: 2 };
        assert!(repack_to_rgba(&[0; 16], 0, 1, 4, layout).is_err());
        assert!(
            repack_to_rgba(&[0; 16], 2, 1, 4, layout).is_err(),
            "stride 小于一行"
        );
        assert!(
            repack_to_rgba(&[0; 3], 1, 1, 4, layout).is_err(),
            "字节数不够一行"
        );
    }

    /// EnumFormat 必须能序列化出一个合法 pod——写错了要在这里炸，不是在用户按快捷键时。
    #[test]
    fn the_enum_format_pod_is_valid() {
        let bytes = enum_format_pod().expect("序列化失败");
        assert!(
            Pod::from_bytes(&bytes).is_some(),
            "序列化出来的不是合法 pod"
        );
    }

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
        let connectors: Vec<String> = monitors
            .iter()
            .map(|(_, connector)| connector.clone())
            .collect();
        let frames = capture_monitors(&connectors).expect("取流会话失败");
        for (connector, frame) in connectors.iter().zip(&frames) {
            let frame = frame.as_ref().expect("取流失败");
            let png = crate::screenshot::encode_png(&frame.rgba, frame.width, frame.height)
                .expect("编码失败");
            let path = std::path::Path::new(&directory).join(format!("frame-{connector}.png"));
            std::fs::write(&path, png).expect("写文件失败");
            println!("{} {}x{}", path.display(), frame.width, frame.height);
        }
    }
}
