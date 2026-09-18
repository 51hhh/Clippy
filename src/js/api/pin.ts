import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  PinCanvasProject,
  PinCanvasSaveMode,
  PinCanvasSaveResult,
  PinImageSharpened,
  PinPayload,
  PinState,
  PinToolbarBounds,
  PinUpdate,
  PinWorkspaceGroup,
  PinWorkspaceStatus,
} from "../ipc-types.ts";

export function pinClip(id: number): Promise<string> {
  return invoke<string>("pin_clip", { id });
}

/** 关闭贴图窗口 */
export function closePin(label: string): Promise<void> {
  return invoke<void>("close_pin", { label });
}

/** 获取统一贴图渲染与交互状态 */
export function getPinPayload(label: string): Promise<PinPayload> {
  return invoke<PinPayload>("get_pin_payload", { label });
}

/**
 * 贴图显示 PNG 的原生资源 URL。revision=0 是首帧（补偿赶上就直接取补偿图，否则取原图），
 * 后台补偿晚到时事件给出更高版本号，避免 WebKit 复用已经解码过的首帧。
 */
export function getPinImageUrl(label: string, revision: number): string {
  return `${convertFileSrc(label, "pin-frame")}?revision=${revision}`;
}

/**
 * 贴图工具条能待的范围（窗口局部逻辑坐标）。
 *
 * 前端算不了这个：它只有 `window.innerWidth`，而贴图窗口的外框永远给工具条留够了位置，
 * 真正会超出屏幕的是窗口在屏幕上的位置——那要问合成器。宽或高为 0 表示查不到，
 * 调用方退回整个窗口。
 */
export function getPinToolbarBounds(label: string): Promise<PinToolbarBounds> {
  return invoke<PinToolbarBounds>("get_pin_toolbar_bounds", { label });
}

/** 贴图内容首帧加载完成后显示原生窗口 */
export function pinReady(label: string): Promise<void> {
  return invoke<void>("pin_ready", { label });
}

/** 更新贴图缩放、透明度或锁定状态 */
/** 应答只带可变字段，不带图片：调用方把它合并进手里的 payload（见 `PinState`） */
export function updatePin(label: string, update: PinUpdate): Promise<PinState> {
  return invoke<PinState>("update_pin", { label, update });
}

/** 复制贴图内容，不触发自动粘贴 */
export function copyPin(label: string): Promise<void> {
  return invoke<void>("copy_pin", { label });
}

export function savePin(label: string): Promise<string> {
  return invoke<string>("save_pin", { label });
}

export function savePinToWorkspace(
  label: string,
  groupId: number | null,
  project: PinCanvasProject | null,
): Promise<PinWorkspaceStatus> {
  return invoke<PinWorkspaceStatus>("save_pin_to_workspace", { label, groupId, project });
}

export function removePinFromWorkspace(label: string): Promise<void> {
  return invoke<void>("remove_pin_from_workspace", { label });
}

export function listPinWorkspaceGroups(): Promise<PinWorkspaceGroup[]> {
  return invoke<PinWorkspaceGroup[]>("list_pin_workspace_groups");
}

export function createPinWorkspaceGroup(name: string): Promise<PinWorkspaceGroup> {
  return invoke<PinWorkspaceGroup>("create_pin_workspace_group", { name });
}

export function renamePinWorkspaceGroup(id: number, name: string): Promise<boolean> {
  return invoke<boolean>("rename_pin_workspace_group", { id, name });
}

export function deletePinWorkspaceGroup(id: number): Promise<boolean> {
  return invoke<boolean>("delete_pin_workspace_group", { id });
}

export function assignPinWorkspaceGroup(label: string, groupId: number | null): Promise<void> {
  return invoke<void>("assign_pin_workspace_group", { label, groupId });
}

/**
 * 贴图的**原图**（base64 PNG），画布导出的底图。
 *
 * 不能用 `getPinPayload` 给的那张：它优先是清晰度补偿版——按缓冲区分辨率渲染、
 * 并为"随后被合成器缩小"预先锐化过，单独看偏大且过冲。导出时单独取一次，
 * 用完即弃（导出是低频动作，不该让每个贴图窗口长期多驻一份原图）。
 */
export function getPinSourceImage(label: string): Promise<string | null> {
  return invoke<string | null>("get_pin_source_image", { label });
}

/**
 * 把贴图上画过的那一版存盘，可选同时进剪贴板。
 *
 * 普通来源条目由 `copyPin`/`savePin` 交付原图；工程来源条目交付保存时的 IDAT 预览。
 * renderer v2 的当前编辑结果只提交工程文档并让后端渲染；`pngBase64` 只保留给 v1 兼容路径。
 * 未修改的导入工程同时传两个 `null`，由后端复用权威 IDAT。
 */
export function savePinCanvas(
  label: string,
  pngBase64: string | null,
  toClipboard: boolean,
  mode: PinCanvasSaveMode,
  project: PinCanvasProject | null,
): Promise<PinCanvasSaveResult> {
  return invoke<PinCanvasSaveResult>("save_pin_canvas", { label, pngBase64, toClipboard, mode, project });
}

/** 由后端按固定 renderer v2 合成并写入剪贴板，不携带工程 iTXt。 */
export function copyPinCanvas(label: string, project: PinCanvasProject): Promise<void> {
  return invoke<void>("copy_pin_canvas", { label, pngBase64: null, project });
}

/** 打开已验证的 Clippy 可编辑 PNG 工程；普通 PNG 由后端拒绝。 */
export function openPinProjectFile(path: string): Promise<string | null> {
  return invoke<string | null>("open_pin_project_file", { path });
}

/**
 * 后台算好的清晰版贴图到货了。
 *
 * 贴图先用原图上屏（开窗才不会被卡住），后端随后在别的线程上把它重新渲染成缓冲区
 * 分辨率并补偿掉合成器的缩小。事件只传资源版本号，PNG 由 WebKit 直接从 `pin-frame`
 * 取走，不再经 base64/JSON/JS 解码。见 `pin/resample.rs`。
 */
export function onPinImageSharpened(
  callback: (payload: PinImageSharpened) => void,
): Promise<UnlistenFn> {
  return listen<PinImageSharpened>("pin-image-sharpened", (event) => callback(event.payload));
}

/**
 * 用户又对同一个条目按了 Pin，而这张图已经贴出来了。
 *
 * 一个条目只对应一个贴图窗口（label 是 GNOME Shell 扩展的查找键，同名开两个会让第二张
 * 摆不了位）。后端因此只把既有窗口显示出来并发这个事件，由前端闪一下外围边框告诉用户
 * "它已经在这儿了"——不然那张贴图被压住或在别的工作区时，用户看到的就是"什么都没发生"。
 */
export function onPinAlreadyOpen(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-already-open", () => callback());
}

export function onPinCurrent(callback: () => void): Promise<UnlistenFn> {
  return listen<null>("pin-current", () => callback());
}
