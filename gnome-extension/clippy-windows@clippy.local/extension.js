// Clippy 的 GNOME Wayland 支撑服务：窗口几何 + 冻结帧截图 + 自己窗口的摆放与置顶。
//
// 为什么需要一个 GNOME Shell 扩展：GNOME Wayland 下客户端拿不到任何窗口的屏幕坐标。
// 实测（GNOME Shell 50，Ubuntu）逐一排除过：
//   - org.gnome.Shell.Introspect.GetWindows —— 只下发 width/height，没有 x/y，
//     而且调用方被 DBusSenderChecker 限定为两个 xdg-desktop-portal 实现；
//   - org.gnome.Shell.Screenshot 整个接口 —— 白名单外调用一律 "is not allowed"；
//   - ext-foreign-toplevel-list-v1 / wlr-foreign-toplevel-management —— 只有标题和
//     app_id，协议里根本没有几何字段，而且 Mutter 两个都不实现；
//   - AT-SPI Component.GetExtents —— 能枚举全部窗口且尺寸正确，但原生 Wayland 窗口
//     的位置一律返回 (0,0)，只有 XWayland 窗口有真坐标；
//   - xcap（X11 枚举）—— 只能看到 XWayland 窗口，本机实测一整个会话只有 0~1 个。
// 唯一持有这份数据的地方就是 gnome-shell 进程自己——它的截图 UI 就在用
// global.get_window_actors() + get_frame_rect()。所以只能以扩展的身份进到进程里取。
//
// 为什么截图也走这里：GNOME Wayland 上 xdg-desktop-portal 的非交互截图要先过一个
// "允许 X 截图吗"的系统对话框，而 gnome-shell 只允许**当前聚焦的应用**弹这个框
// （实测报错 "Only the focused app is allowed to show a system access dialog"）。
// 截图是全局快捷键触发的，那一刻 Clippy 没有任何窗口聚焦，于是对话框永远弹不出来、
// 非交互截图永远失败，只能退到 interactive 模式——那玩意就是 GNOME 自带的截图界面，
// 用户按下 Clippy 的快捷键却看到系统截图 UI。Shell.Screenshot 在扩展里可以直接用，
// 不需要对话框、不落文件到用户的图片目录、也没有系统截图那一下闪白。
//
// 为什么窗口摆放也走这里：Wayland 里客户端无权决定自己窗口的位置，也无权把自己置顶，
// Mutter 把 gtk_window_move / set_keep_above 全部静默忽略。贴图窗口要回到截图时的原位、
// 并且压在别的窗口上面，只能由 Shell 内部调 MetaWindow.move_frame() / make_above()。
//
// 安全：窗口标题会泄露用户正在做什么、截图更是整屏内容，所以这两个接口不对所有本地
// 进程开放。调用方必须出示 Clippy 写在扩展目录里的 0600 令牌文件内容。同用户的普通
// 进程当然读得到那个文件（同用户之间本来就没有边界），但沙箱应用（Flatpak/Snap）
// 通常只有 session bus 而没有 $HOME 读权限，令牌能把这类调用挡在外面。

import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Mtk from 'gi://Mtk';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const IFACE = `
<node>
  <interface name="org.gnome.Shell.Extensions.ClippyWindows">
    <method name="GetVersion">
      <arg type="u" direction="out" name="version"/>
    </method>
    <method name="GetWindows">
      <arg type="s" direction="in" name="token"/>
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="Screenshot">
      <arg type="s" direction="in" name="token"/>
      <arg type="s" direction="out" name="path"/>
    </method>
    <method name="ScreenshotArea">
      <arg type="s" direction="in" name="token"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="width"/>
      <arg type="i" direction="in" name="height"/>
      <arg type="s" direction="out" name="path"/>
    </method>
    <method name="CaptureArea">
      <arg type="s" direction="in" name="token"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="i" direction="in" name="width"/>
      <arg type="i" direction="in" name="height"/>
      <arg type="d" direction="in" name="scale"/>
      <arg type="s" direction="out" name="path"/>
      <arg type="i" direction="out" name="pixelWidth"/>
      <arg type="i" direction="out" name="pixelHeight"/>
      <arg type="i" direction="out" name="stride"/>
      <arg type="s" direction="out" name="format"/>
    </method>
    <method name="PlaceWindow">
      <arg type="s" direction="in" name="token"/>
      <arg type="u" direction="in" name="pid"/>
      <arg type="s" direction="in" name="marker"/>
      <arg type="i" direction="in" name="x"/>
      <arg type="i" direction="in" name="y"/>
      <arg type="b" direction="in" name="reposition"/>
      <arg type="b" direction="in" name="above"/>
      <arg type="b" direction="out" name="placed"/>
    </method>
  </interface>
</node>`;

const OBJECT_PATH = '/org/gnome/Shell/Extensions/ClippyWindows';

/// 载荷格式版本。加接口或改 GetWindows 的 JSON 字段时加一，Rust 侧据此判断能力。
/// gnome-shell 只在登录时加载扩展（ReloadExtension 实测已废弃，直接报
/// "is deprecated and does not work"），所以升级后磁盘上是新版、跑着的是旧版，
/// 这个版本号就是区分两者的唯一依据。
const PROTOCOL_VERSION = 5;

/// 截图落地目录名，挂在 XDG_RUNTIME_DIR 下（tmpfs、0700、注销即清）。
/// 必须与 Rust 侧 shell_extension.rs 的 SCREENSHOT_DIR_NAME 一致。
const SCREENSHOT_DIR_NAME = 'clippy-shots';

/// 令牌文件名，和 Rust 侧 shell_extension.rs 的 TOKEN_FILE_NAME 必须一致。
const TOKEN_FILE_NAME = 'token';

/// 令牌长度下限，防止空文件或被截断的文件被当成有效凭据。
const MIN_TOKEN_LENGTH = 16;

export default class ClippyWindowsExtension extends Extension {
    enable() {
        this._dbus = Gio.DBusExportedObject.wrapJSObject(IFACE, this);
        this._dbus.export(Gio.DBus.session, OBJECT_PATH);
    }

    disable() {
        this._dbus?.unexport();
        this._dbus = null;
    }

    GetVersion() {
        return PROTOCOL_VERSION;
    }

    GetWindows(token) {
        if (!this._tokenMatches(token)) {
            throw Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.AccessDenied',
                'Clippy window geometry requires a matching token');
        }

        const workspace = global.workspace_manager.get_active_workspace();
        // sort_windows_by_stacking 返回由下到上，反转成"索引 0 是最上层"——和
        // window_probe.rs 下发候选的约定一致，覆盖层取第一个命中的窗口。
        const stacked = global.display
            .sort_windows_by_stacking(global.get_window_actors().map(actor => actor.meta_window))
            .reverse();

        const windows = [];
        for (const window of stacked) {
            if (window.is_override_redirect() || window.minimized)
                continue;
            if (!window.located_on_workspace(workspace))
                continue;
            // 和 Shell 自己的截图 UI 用同一套类型过滤，免得把顶栏、OSD 之类算成候选。
            const type = window.get_window_type();
            if (type !== Meta.WindowType.NORMAL &&
                type !== Meta.WindowType.DIALOG &&
                type !== Meta.WindowType.MODAL_DIALOG &&
                type !== Meta.WindowType.UTILITY)
                continue;

            // frame_rect 是逻辑像素、且不含 CSD 阴影——正好是肉眼看到的那个矩形，
            // 所以 Rust 侧不用再做 X11 那条路的缩放折算和 _GTK_FRAME_EXTENTS 裁边。
            const rect = window.get_frame_rect();
            windows.push({
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                title: window.get_title() ?? '',
                wm_class: window.get_wm_class() ?? '',
                pid: window.get_pid(),
            });
        }
        return JSON.stringify(windows);
    }

    /**
     * 截整个 stage（含全部显示器）到一个私有 PNG，返回路径。
     *
     * 异步：Shell.Screenshot 是异步 API，而且截图慢过一次 D-Bus 往返也不该卡住
     * gnome-shell 的主循环。GDBusExportedObject.wrapJSObject 认 `<Method>Async`
     * 这个命名约定，参数由 invocation 回复。
     *
     * 不闪白、不落文件到用户的图片目录：闪白是 Shell 截图 UI 自己加的效果，
     * 保存到 ~/Pictures 是 xdg-desktop-portal 的行为，这里两者都没有。
     *
     * **混合缩放的多屏上这条路会把画面弄糊**，原因见 ScreenshotAreaAsync。
     * 它现在只是那条路的兜底（Rust 侧协议低于 v4、或逐屏截图失败时才用）。
     */
    ScreenshotAsync(params, invocation) {
        const [token] = params;
        if (!this._tokenMatches(token)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.AccessDenied',
                'Clippy screenshot requires a matching token'));
            return;
        }

        // 不含光标：冻结帧是给覆盖层当底图的，烧进去一个光标只会碍事。
        this._captureToFile(invocation,
            (shooter, stream, done) => shooter.screenshot(false, stream, done),
            (shooter, result) => shooter.screenshot_finish(result));
    }

    /**
     * 截 stage 上的**一块矩形区域**（逻辑像素，与 GetWindows 同一坐标系）到私有 PNG。
     *
     * 为什么需要它——这是"截图很糊"的根因所在。Mutter 算截图尺寸用的是
     * `clutter_stage_get_capture_final_size`：`区域 × max(与该区域相交的各视图的缩放)`。
     * 整个 stage 的矩形当然和每块屏都相交，于是**整张图都按全桌面最大的那个缩放渲染**，
     * 缩放较低的屏在图里是被上采样的。实测本机：eDP 原生 2560x1600（逻辑 1920x1200，
     * 缩放 1.3333）在整屏图里是 2880x1800（= 逻辑 × 外接 4K 的 1.5），糊在最开始那一步，
     * 后面无论怎么裁都救不回来。把区域收成**单块屏的逻辑矩形**之后，相交的只有这块屏
     * 自己的视图，max(scale) 就是它自己的缩放，出来的正好是原生像素。
     *
     * 区域必须**正好是**那块屏的矩形，不要向外留边：多出去的一个像素就会碰到隔壁屏的
     * 视图，把它的缩放重新拉进 max() 里，等于又回到上采样。
     *
     * 每次都 `new Shell.Screenshot()`：一个 ShellScreenshot 实例同时只允许一次截图
     * （第二次直接 G_IO_ERROR_PENDING "Only one screenshot operation at a time"），
     * 而逐屏截图要的正是它们同时进行——PNG 编码在各自的 worker 线程里跑，能真正重叠。
     *
     * 区域截图不含光标（shell_screenshot_screenshot_area 压根没有这个参数），
     * 和整屏那条路一致。
     */
    ScreenshotAreaAsync(params, invocation) {
        const [token, x, y, width, height] = params;
        if (!this._tokenMatches(token)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.AccessDenied',
                'Clippy screenshot requires a matching token'));
            return;
        }
        // 空矩形会让 Clutter 那边直接失败，报错还很难懂；在门口挡掉。
        if (!(width > 0) || !(height > 0)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.InvalidArgs',
                `Clippy screenshot area ${width}x${height} is empty`));
            return;
        }

        this._captureToFile(invocation,
            (shooter, stream, done) => shooter.screenshot_area(x, y, width, height, stream, done),
            (shooter, result) => shooter.screenshot_area_finish(result));
    }

    /**
     * 截一块区域，**不编码成 PNG**，直接把原始 RGBA 字节落进私有文件。
     *
     * 这是"截图慢"的正解。实测（GNOME 50.1，双屏，Intel Arc iGPU）把区域截图拆开量：
     * 同样 4.1 Mpx，内容简单的 eDP 要 124 ms、PNG 640 KiB；内容复杂的 4K 屏要 884 ms、
     * PNG 3217 KiB。**像素数一样、时间差 7 倍、字节差 5 倍**——差的那 760 ms 全在
     * PNG 的 deflate 上，而 `Shell.Screenshot` 没有任何调压缩级别的入口。整屏 8.3 Mpx
     * 的 4K 因此要 1.7 秒，占了整条截图链路的绝大部分。
     *
     * 于是绕开编码器：`paint_to_content` 把区域画进一张纹理，`get_data` 读回原始像素，
     * 写文件（XDG_RUNTIME_DIR 是 tmpfs，等于写内存）。Rust 侧读到的就是能直接当冻结帧
     * 用的 RGBA，两头各省一次编解码。
     *
     * **`scale` 由调用方指定**，这比 `ScreenshotArea` 更可靠：那条路的尺寸由 Mutter 算成
     * `区域 × max(相交视图的缩放)`，得靠"区域正好等于单块屏"才拿得到原生像素；这里直接
     * 传这块屏自己的缩放，与相交视图无关。
     *
     * 尺寸取**纹理自己报的宽高**，不去复刻 Mutter 的取整（`ceilf` 还是 `round` 属于实现
     * 细节，猜错一个像素整张图就斜了）。stride 也一并回给 Rust，让它自己决定要不要重排行。
     *
     * 任何一步抛异常都由调用者退回 `screenshot_area` 的 PNG 路径——慢，但画面是对的。
     */
    // stage 的色彩状态。`paint_to_content` 的 color_state 允许为 null（typelib 里标了
    // nullable），而 `get_color_state` 是从 Clutter.Actor 继承来的、并非每个版本都有，
    // 所以拿不到就传 null，不要让整条路死在一个 getter 上。
    _stageColorState() {
        try {
            return global.stage.get_color_state?.() ?? null;
        } catch (_error) {
            return null;
        }
    }

    _captureAreaToRawFile(x, y, width, height, scale) {
        const content = global.stage.paint_to_content(
            new Mtk.Rectangle({x, y, width, height}),
            scale,
            this._stageColorState(),
            // 不含光标：冻结帧是覆盖层的底图，烧进一个光标只会碍事。
            Clutter.PaintFlag.NO_CURSORS);
        if (typeof content?.get_texture !== 'function')
            throw new Error(`paint_to_content gave ${content}`);
        const texture = content.get_texture();
        const pixelWidth = texture.get_width();
        const pixelHeight = texture.get_height();
        if (!(pixelWidth > 0) || !(pixelHeight > 0))
            throw new Error(`texture is ${pixelWidth}x${pixelHeight}`);
        if (!texture.is_get_data_supported())
            throw new Error('this texture cannot be read back');

        const stride = pixelWidth * 4;
        const data = new Uint8Array(stride * pixelHeight);
        // 埋哨兵再读回。`get_data` 的 data 参数在 typelib 里是没有长度标注的 array<u8>，
        // 万一 GJS 把 Uint8Array **复制**一份交给 Cogl，Cogl 写的是那份副本，我们手里
        // 这份仍是原样——那样得到的是一张全黑的图，而不是一个能被 catch 的异常。
        // 32 个哨兵全都没被覆盖才判定失败，误判概率 (1/256)^32，实际为零。
        const probes = [];
        const step = Math.max(1, Math.floor(data.length / 32));
        for (let offset = 0; offset < data.length; offset += step) {
            data[offset] = 0xCD;
            probes.push(offset);
        }
        // 直接要非预乘的 RGBA，省掉 Rust 侧的还原：截图内容 alpha 恒为 255，
        // 预乘与否本无差别，但把格式钉死能让前端那份"RGBA8 行优先"的契约不打折扣。
        const copied = texture.get_data(Cogl.PixelFormat.RGBA_8888, stride, data);
        if (!(copied > 0))
            throw new Error(`Cogl copied ${copied} bytes`);
        if (probes.every(offset => data[offset] === 0xCD))
            throw new Error('the pixel buffer never reached Cogl');

        const [path, stream] = this._createScreenshotStream('rgba');
        try {
            stream.write_all(data, null);
            stream.close(null);
        } catch (error) {
            try {
                stream.close(null);
            } catch (_closeError) {
                // 已经关掉或写坏了都无所谓，下面照样删文件。
            }
            GLib.unlink(path);
            throw error;
        }
        return [path, pixelWidth, pixelHeight, stride, 'RGBA'];
    }

    /**
     * 逐屏取原始像素，失败就退回 `ScreenshotArea` 的 PNG。返回
     * `(路径, 像素宽, 像素高, stride, 格式)`；PNG 那条路的宽高与 stride 是 0，
     * 由 Rust 侧从文件头读——反正它本来就要解这张图。
     */
    CaptureAreaAsync(params, invocation) {
        const [token, x, y, width, height, scale] = params;
        if (!this._tokenMatches(token)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.AccessDenied',
                'Clippy screenshot requires a matching token'));
            return;
        }
        if (!(width > 0) || !(height > 0)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.InvalidArgs',
                `Clippy capture area ${width}x${height} is empty`));
            return;
        }
        if (!(scale > 0)) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.InvalidArgs',
                `Clippy capture scale ${scale} is not positive`));
            return;
        }

        try {
            invocation.return_value(new GLib.Variant('(siiis)',
                this._captureAreaToRawFile(x, y, width, height, scale)));
            return;
        } catch (error) {
            // 只在这里打日志：真跑起来才知道这套 GI 编排在哪个 Shell 版本上不通，
            // 而 journal 里这一行就是唯一线索。退回 PNG 只是慢，画面仍然是对的。
            console.warn(`Clippy raw capture failed, falling back to PNG: ${error}`);
        }

        this._captureToFile(invocation,
            (shooter, stream, done) => shooter.screenshot_area(x, y, width, height, stream, done),
            (shooter, result) => shooter.screenshot_area_finish(result),
            path => new GLib.Variant('(siiis)', [path, 0, 0, 0, 'PNG']));
    }

    /// 三个 PNG 截图方法共用的收尾：开一个私有文件、发起截图、成败都把文件处置干净。
    /// `reply` 决定怎么把路径包成应答——`CaptureArea` 的出参比另两个多。
    _captureToFile(invocation, start, finish, reply = path => new GLib.Variant('(s)', [path])) {
        let path, stream;
        try {
            [path, stream] = this._createScreenshotStream();
        } catch (error) {
            invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.Failed',
                `Clippy screenshot could not open a target file: ${error.message}`));
            return;
        }

        start(new Shell.Screenshot(), stream, (shooter, result) => {
            try {
                finish(shooter, result);
                stream.close(null);
                invocation.return_value(reply(path));
            } catch (error) {
                // 失败就别把空文件留在 runtime dir 里：Rust 侧只会删自己读到的那个路径。
                try {
                    stream.close(null);
                } catch (_closeError) {
                    // 已经关掉或写坏了都无所谓，下面照样删文件。
                }
                GLib.unlink(path);
                invocation.return_gerror(Gio.DBusError.new_for_dbus_error(
                    'org.freedesktop.DBus.Error.Failed',
                    `Clippy screenshot failed: ${error.message}`));
            }
        });
    }

    /**
     * 把调用方自己的某个窗口摆到指定位置、并/或置顶。找不到窗口返回 false。
     *
     * 为什么这件事也得进 gnome-shell：Wayland 协议里客户端无权决定自己窗口的位置
     * （xdg_toplevel 只描述内容），`gtk_window_move` / `gtk_window_set_keep_above`
     * 在 Mutter 下都是静默空操作。于是贴图窗口既回不到截图时的原位，也压不住别的窗口。
     * 只有 Shell 内部的 MetaWindow 有 move_frame() 和 make_above()。
     *
     * `marker` 是窗口标题——贴图窗口无装饰、不进任务栏，标题只作为这里的查找键。
     * `pid` 把作用范围收在调用方自己的窗口上：调用方报什么 pid 我们不能核实
     * （wrapJSObject 的同步方法拿不到 sender 凭据），所以这是**限定作用域**而不是
     * 安全边界——真正的边界是令牌，而持有令牌的进程本来就能截整屏。
     *
     * 坐标是逻辑像素、指的是 frame rect（不含 CSD 阴影），与 GetWindows 同一坐标系。
     */
    PlaceWindow(token, pid, marker, x, y, reposition, above) {
        if (!this._tokenMatches(token)) {
            throw Gio.DBusError.new_for_dbus_error(
                'org.freedesktop.DBus.Error.AccessDenied',
                'Clippy window placement requires a matching token');
        }
        if (typeof marker !== 'string' || marker.length === 0)
            return false;

        const window = global.get_window_actors()
            .map(actor => actor.meta_window)
            .find(candidate => candidate.get_pid() === pid &&
                (candidate.get_title() ?? '') === marker);
        if (!window)
            return false;

        // 置顶先做：move_frame 之后再 make_above 会让窗口先在旧层闪一下。
        if (above)
            window.make_above();
        else
            window.unmake_above();
        // user_op = false：这不是用户拖窗口，不该打断 Mutter 的贴边/平铺状态记忆。
        if (reposition)
            window.move_frame(false, x, y);
        return true;
    }

    /// 在 XDG_RUNTIME_DIR 下开一个 0600 的截图文件，返回 [路径, 输出流]。
    /// 后缀区分内容：`png` 是编码过的，`rgba` 是原始像素（Rust 侧据此决定要不要解码）。
    _createScreenshotStream(suffix = 'png') {
        const directory = GLib.build_filenamev([
            GLib.get_user_runtime_dir(), SCREENSHOT_DIR_NAME]);
        const folder = Gio.File.new_for_path(directory);
        try {
            folder.make_directory_with_parents(null);
        } catch (error) {
            if (!error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.EXISTS))
                throw error;
        }
        const path = GLib.build_filenamev([
            directory, `frame-${GLib.uuid_string_random()}.${suffix}`]);
        const file = Gio.File.new_for_path(path);
        // PRIVATE = 0600。整屏画面不该让同机器的别人读到。
        const stream = file.replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
        return [path, stream];
    }

    _tokenMatches(token) {
        if (typeof token !== 'string' || token.length < MIN_TOKEN_LENGTH)
            return false;
        const expected = this._readToken();
        return expected !== null && expected === token;
    }

    _readToken() {
        const path = GLib.build_filenamev([this.path, TOKEN_FILE_NAME]);
        try {
            // 每次调用都重读：Clippy 轮换令牌后不需要注销重启 Shell。
            const [ok, bytes] = Gio.File.new_for_path(path).load_contents(null);
            if (!ok)
                return null;
            const value = new TextDecoder().decode(bytes).trim();
            return value.length >= MIN_TOKEN_LENGTH ? value : null;
        } catch (_error) {
            // 文件不存在就是"没授权过"，属于正常状态，不打日志刷屏。
            return null;
        }
    }
}
