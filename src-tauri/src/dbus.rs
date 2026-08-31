//! 阻塞式 D-Bus 调用的唯一入口。**不要在别处直接用 `zbus::blocking`。**
//!
//! 原因：ashpd 打开了 zbus 的 `tokio` feature，于是 `zbus::blocking` 内部拿一个静态的
//! 多线程 tokio runtime 做 `block_on`；而 tokio 不允许在已经进入 runtime 的线程上再启动
//! 一个 runtime，于是直接 panic —— `Cannot start a runtime from within a runtime`。
//!
//! 三种线程实测（`blocking_calls_survive_inside_an_async_task` 把它钉住了）：
//!
//! | 调用线程 | 结果 |
//! |---|---|
//! | tokio async worker（`#[tauri::command] async fn` 的函数体） | **panic** |
//! | `spawn_blocking` 线程 | 侥幸能过 |
//! | 普通 OS 线程 | 正常 |
//!
//! 也就是说，同一个同步函数在 `spawn_blocking` 里调没事、被 async 命令直接调就炸，
//! 而 `probe()` / `status()` 这类函数两边都有调用方——靠"记得别在 async 里调"是守不住的。
//! 所以这里统一先跳到一条干净的 OS 线程再开连接：多花一次线程创建（几十微秒），
//! 换来调用方不必关心自己跑在什么线程上。

//! ## 连接是复用的
//!
//! `Connection::session()` 每次都要重新做一遍 SASL 握手加 `Hello`，实测一次约 1~2 ms。
//! 单看不多，但贴图窗口每缩放一帧都会调 `PlaceWindow`（还要先探一次 `GetVersion`），
//! 而 `update_pin` 是同步命令、跑在**主线程**上，于是这点开销直接落在 UI 的响应上。
//! 所以 session 连接建一次就缓存起来（`Connection` 内部是 `Arc`，克隆很便宜），
//! 只有在连接真的坏掉时才丢掉重连——判据见 `worth_reconnecting`。

use std::sync::{Mutex, OnceLock};
use zbus::blocking::{Connection, Proxy};

/// 跑一次阻塞的 D-Bus 方法调用，返回反序列化后的应答体。
pub(crate) fn call<Args, Ret>(
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    args: &Args,
) -> Result<Ret, zbus::Error>
where
    Args: serde::Serialize + zbus::zvariant::DynamicType + Sync,
    Ret: for<'d> zbus::zvariant::DynamicDeserialize<'d> + Send,
{
    with_session(|connection| {
        Proxy::new(connection, destination, path, interface)?.call(method, args)
    })
}

/// 读一个 D-Bus 属性。
pub(crate) fn property<Ret>(
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Result<Ret, zbus::Error>
where
    Ret: TryFrom<zbus::zvariant::OwnedValue> + Send,
    <Ret as TryFrom<zbus::zvariant::OwnedValue>>::Error: Into<zbus::Error>,
{
    with_session(|connection| {
        Proxy::new(connection, destination, path, interface)?.get_property(name)
    })
}

fn session_cache() -> &'static Mutex<Option<Connection>> {
    static CACHE: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// 在缓存的 session 连接上执行 `work`，连接坏了就丢掉重连并重试一次。
///
/// `work` 必须可重入（只有 `Fn` 才能被调第二次）：这里唯一会重跑它的情况是"请求根本
/// 没送达对端"，所以重试不会把一次成功的副作用做两遍。
fn with_session<T: Send>(
    work: impl Fn(&Connection) -> Result<T, zbus::Error> + Send,
) -> Result<T, zbus::Error> {
    off_async_runtime(move || {
        let cached = session_cache().lock().ok().and_then(|slot| slot.clone());
        if let Some(connection) = cached {
            match work(&connection) {
                Err(error) if worth_reconnecting(&error) => {
                    log::debug!("session D-Bus 连接失效，重连一次：{error}");
                    if let Ok(mut slot) = session_cache().lock() {
                        *slot = None;
                    }
                }
                result => return result,
            }
        }
        let connection = Connection::session()?;
        let result = work(&connection);
        if worth_caching(&result) {
            if let Ok(mut slot) = session_cache().lock() {
                *slot = Some(connection);
            }
        }
        result
    })
}

/// 新建的连接值不值得留在缓存里。
///
/// 判据与丢弃缓存的判据必须是同一个（`worth_reconnecting` 的反面）：业务错误
/// （`MethodError`）恰恰证明连接是通的。只按 `result.is_ok()` 判会把这种连接一起扔掉，
/// 于是"插件没装 / 版本不对"这类**每次都失败**的探测，每一次都要重新做一遍
/// SASL 握手（约 1~2 ms），而这条路上有跑在主线程的调用方。
fn worth_caching<T>(result: &Result<T, zbus::Error>) -> bool {
    match result {
        Ok(_) => true,
        Err(error) => !worth_reconnecting(error),
    }
}

/// 只有"连接本身出问题"才值得丢掉缓存重连。
///
/// `MethodError` 是对端**正常应答**的一个错误（令牌不对、方法不存在……），它恰好证明连接是
/// 通的；把它当成连接故障会让每一次业务错误都白搭一次握手，还会把有副作用的方法重发一遍。
fn worth_reconnecting(error: &zbus::Error) -> bool {
    !matches!(error, zbus::Error::MethodError(..))
}

/// 把一段阻塞调用挪到一条干净的 OS 线程上执行，避开调用方线程上的 tokio runtime 上下文。
///
/// 用 scoped thread 是为了让闭包能直接借用调用方的 `&str` 与参数，不必为了 `'static`
/// 把每个参数都克隆一遍。线程内的 panic 原样抛回调用方，不吞掉。
fn off_async_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| match scope.spawn(work).join() {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 复刻 `zbus::blocking` 的做法：内部再建一个多线程 runtime 做 `block_on`。
    fn nested_block_on() -> u32 {
        tokio::runtime::Builder::new_multi_thread()
            .build()
            .expect("创建嵌套 runtime 失败")
            .block_on(async { 42 })
    }

    /// 这个模块存在的全部理由，两个方向都要钉住：直接在 async 任务里调必炸，
    /// 跳一次线程就没事。真正的 D-Bus 调用没法进 CI（没有 session bus），
    /// 但会炸的从来不是 D-Bus 本身，而是这层嵌套 runtime。
    #[test]
    fn blocking_calls_survive_inside_an_async_task() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("创建测试 runtime 失败");

        // 预期内的 panic，别让它的默认输出污染测试日志；立刻把 hook 装回去。
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let unguarded = runtime.block_on(async { std::panic::catch_unwind(nested_block_on) });
        std::panic::set_hook(previous_hook);
        assert!(
            unguarded.is_err(),
            "async 任务里的嵌套 block_on 竟然没炸——如果 zbus 换掉了 block_on 的实现，\
             这个模块就可以简化了，但别在没确认之前删掉它"
        );

        assert_eq!(
            runtime.block_on(async { off_async_runtime(nested_block_on) }),
            42
        );
    }

    /// 造一个对端应答的业务错误（令牌不对、方法不存在……）。
    fn method_error() -> zbus::Error {
        let call = zbus::message::Message::method_call("/org/example", "Method")
            .and_then(|builder| builder.interface("org.example.Iface"))
            .and_then(|builder| builder.destination("org.example"))
            .map(|builder| builder.serial(std::num::NonZeroU32::new(1).expect("非零")))
            .and_then(|builder| builder.build(&()))
            .expect("构造方法调用失败");
        let denied = zbus::message::Message::error(&call.header(), "org.example.AccessDenied")
            .and_then(|builder| builder.build(&"token mismatch"))
            .expect("构造错误应答失败");
        zbus::Error::from(denied)
    }

    fn broken_pipe() -> zbus::Error {
        zbus::Error::InputOutput(std::sync::Arc::new(std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        )))
    }

    /// 缓存连接的重连判据：对端应答的业务错误不能触发重连，否则每次令牌校验失败都要
    /// 白搭一次握手，而且有副作用的方法（`Screenshot`）会被发第二遍。
    #[test]
    fn only_broken_connections_are_worth_reconnecting() {
        assert!(!worth_reconnecting(&method_error()));
        assert!(worth_reconnecting(&broken_pipe()));
    }

    /// 存入缓存的判据必须是重连判据的反面。只按"调用成功"判会把一条被业务错误
    /// 拒绝、但本身完好的连接扔掉，于是每次失败的探测都要重新握手一遍。
    #[test]
    fn a_connection_rejected_by_a_method_error_is_still_cached() {
        assert!(worth_caching::<()>(&Ok(())));
        assert!(worth_caching::<()>(&Err(method_error())));
        assert!(!worth_caching::<()>(&Err(broken_pipe())));
    }

    /// 线程里的 panic 不能被悄悄吞掉，否则真正的 bug 会变成一个莫名其妙的默认值。
    #[test]
    #[should_panic(expected = "故意炸的")]
    fn panics_inside_the_worker_thread_propagate() {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(|| off_async_runtime(|| panic!("故意炸的")));
        std::panic::set_hook(previous_hook);
        std::panic::resume_unwind(result.expect_err("本该 panic"));
    }
}
