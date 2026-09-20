import { useCallback, useEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { getCurrentWindowLabel, startDraggingCurrentWindow } from "../../js/api.ts";
import { pinApi } from "./api";
import {
  allowsTextSelection,
  type DragTracking,
  isZoomShortcut,
  NO_DRAG,
  pinWheelIntent,
  trackDragMove,
  trackDragPointerDown,
} from "./gestures";
import { PinCanvasToolbar } from "./PinCanvasToolbar";
import { PinContextMenu, type PinMenuItem } from "./PinContextMenu";
import { PinToolbar } from "./PinToolbar";
import { PinSaveDialog } from "./PinSaveDialog";
import { PinWorkspacePopover } from "./PinWorkspacePopover";
import { pinImageRendering } from "./rendering";
import type { PinPayload, PinUpdate, PinWorkspaceGroup, PlatformCapability } from "./types";
import { mergePinState, shouldApplyPinUpdateResponse } from "./update-order";
import { usePinCanvas } from "./usePinCanvas";
import { usePinToolbarBounds } from "./usePinToolbarBounds";
import { isToolbarDragging } from "../shared/useToolbarDrag";
import { t } from "../shared/i18n";
import { isColorPinPayload } from "./colorPayload";

/** 事件落点是不是工具条/滑块那一片。 */
function onPinControls(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("[data-pin-controls]") !== null;
}

/** 内容区相对窗口原点的偏移，与 `pin.css` 的 `.pin-media` inset 和 `pin/window.rs` 的
 *  `SHADOW_GUTTER` 是一份契约。工具条要按内容区的位置选边，所以这里要知道它。 */
const MEDIA_INSET = 12;

/** 同一次 mount 的服务保持稳定；生产入口省略该参数。 */
export type PinAppServices = {
  api: typeof pinApi;
  windowLabel: () => string;
  startDragging: () => Promise<void>;
  viewport?: () => { width: number; height: number };
};
const defaultServices: PinAppServices = { api: pinApi, windowLabel: getCurrentWindowLabel, startDragging: startDraggingCurrentWindow };

export function App({ services = defaultServices }: { services?: PinAppServices } = {}) {
  const pinApi = services.api;
  const label = services.windowLabel();
  const drag = useRef<DragTracking>(NO_DRAG);
  const wheelFrame = useRef<number | null>(null);
  const pendingUpdate = useRef<PinUpdate>({});
  const updateGeneration = useRef(0);
  const copiedTimer = useRef<number | null>(null);
  const pinRef = useRef<PinPayload | null>(null);
  const confirmedPinRef = useRef<PinPayload | null>(null);
  const confirmedGeneration = useRef(0);
  const [pin, setPin] = useState<PinPayload | null>(null);
  const [alwaysOnTop, setAlwaysOnTop] = useState<PlatformCapability | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [copied, setCopied] = useState(false);
  const [opacityOpen, setOpacityOpen] = useState(false);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [workspaceGroups, setWorkspaceGroups] = useState<PinWorkspaceGroup[]>([]);
  const [workspaceBusy, setWorkspaceBusy] = useState(false);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pixelSize, setPixelSize] = useState<{ width: number; height: number } | null>(null);
  const [reminding, setReminding] = useState(false);
  const remindTimer = useRef<number | null>(null);
  const [canvasOpen, setCanvasOpen] = useState(false);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  /** 关窗时"要不要保存画布"的询问。`null` 表示没在问。 */
  const [closePrompt, setClosePrompt] = useState(false);
  const [closing, setClosing] = useState(false);
  const closingRef = useRef(false);
  const [savePrompt, setSavePrompt] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [noticeKey, setNoticeKey] = useState("pin.saved");
  const savedTimer = useRef<number | null>(null);
  const imageElement = useRef<HTMLImageElement | null>(null);
  const [viewport, setViewport] = useState(() => services.viewport?.() ?? {
    width: window.innerWidth,
    height: window.innerHeight,
  });
  const updateInFlight = useRef(false);
  // `flushUpdate` 要在自己的 finally 里再排一次，而 `scheduleFlush` 又要调它，
  // 两个 useCallback 互相依赖成环。用一个 ref 打破环，rAF 里读到的永远是最新那个。
  const flushRef = useRef<() => void>(() => {});

  const scheduleFlush = useCallback(() => {
    if (wheelFrame.current !== null) return;
    wheelFrame.current = requestAnimationFrame(() => {
      wheelFrame.current = null;
      flushRef.current();
    });
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    pinApi
      // 先挂清晰图监听，再请求 payload。这样首帧资源已经取走、后台恰好在同一刻完成时，
      // revision=1 事件也不会掉进“图片已 served、监听还没挂上”的窄缝。
      .onSharpened((payload) => {
        // 监听先于 payload 请求挂上；只有已确认的 image Pin 才能生成 pin-frame URL。
        if (payload.label !== label || pinRef.current?.kind !== "image") return;
        setImageUrl(pinApi.imageUrl(label, payload.revision));
      })
      .catch((reason) => {
        // 清晰度补偿是增强项；监听挂不上仍应继续显示首帧，不能把整张贴图关掉。
        console.error(reason);
        return () => {};
      })
      .then((stop) => {
        if (cancelled) {
          stop();
          return null;
        }
        unlisten = stop;
        return pinApi.get(label);
      })
      .then((payload) => {
        if (!cancelled && payload) {
          // 颜色 payload 来自 IPC，不能相信 TypeScript 声明。校验失败时不写 state，
          // 因此不会建立内容 DOM 或含不可信值的 CSS；直接关闭隐藏原生窗口。
          if (payload.kind === "color" && !isColorPinPayload(payload)) {
            console.error("Invalid color pin payload");
            void pinApi.close(label).catch(() => undefined);
            return;
          }
          if (payload.kind === "image") setImageUrl(pinApi.imageUrl(label, 0));
          pinRef.current = payload;
          confirmedPinRef.current = payload;
          setPin(payload);
        }
      })
      .catch((reason) => {
        if (!cancelled) {
          console.error(reason);
          setError(t("pin.loadFailed"));
          // Keep a failed payload window hidden; showing it would produce an
          // empty pin and leave a native window behind the error state.
          void pinApi.close(label).catch(() => undefined);
        }
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [label]);

  useEffect(() => {
    let cancelled = false;
    pinApi
      .platform()
      .then((platform) => {
        if (!cancelled) setAlwaysOnTop(platform.capabilities.always_on_top);
      })
      .catch((reason) => console.error(reason));
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    pinRef.current = pin;
  }, [pin]);

  /**
   * 用户又对同一个条目按了 Pin：闪一下外围边框说明"它已经在这儿了"。
   *
   * 动画靠加一个 class 驱动，所以连按两次要先摘掉再挂上，否则第二次不会重新播放。
   * 时长比 `pin-remind` 的 1.1s 略长一点，等动画自己走完。
   */
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    pinApi
      .onAlreadyOpen(() => {
        if (remindTimer.current !== null) window.clearTimeout(remindTimer.current);
        // Tauri 事件不属于 React 合成事件；若 React 把摘 class 与下一帧重新添加合并，
        // 浏览器看不到中间状态，连续按 Pin 时 CSS 动画不会重播。先同步提交摘除再等帧。
        flushSync(() => setReminding(false));
        requestAnimationFrame(() => setReminding(true));
        remindTimer.current = window.setTimeout(() => {
          remindTimer.current = null;
          setReminding(false);
        }, 1200);
      })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch((reason) => console.error(reason));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /** 工具条要按可见视口选边，窗口缩放（滚轮改 scale）时它会变。 */
  useEffect(() => {
    function onResize() {
      setViewport(services.viewport?.() ?? { width: window.innerWidth, height: window.innerHeight });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (!pin || ready || (pin.kind === "image" && !imageUrl)) return;
    if (pin.kind === "text" || pin.kind === "color") {
      requestAnimationFrame(() => setReady(true));
    }
  }, [imageUrl, pin, ready]);

  useEffect(() => {
    if (!ready) return;
    pinApi.ready(label).catch((reason) => {
      console.error(reason);
      setError(t("pin.actionFailed"));
    });
  }, [label, ready]);

  /**
   * 把攒下的那一份改动发出去。
   *
   * **同一时刻只允许一个请求在飞。** `update_pin` 在工作线程协调原生窗口操作，
   * 其中仍有 UI 调度和摆位的 D-Bus 往返；按 rAF 无条件发会持续积压待处理请求。
   * 改成"在飞就先攒着，落地了再发最新的一份"之后，
   * 发送频率自动跟上后端的处理能力，而本地 CSS 仍然每帧都跟手（乐观更新那一段没变）。
   */
  const flushUpdate = useCallback(() => {
    if (updateInFlight.current) return;
    const next = pendingUpdate.current;
    if (Object.keys(next).length === 0) return;
    pendingUpdate.current = {};
    updateInFlight.current = true;
    const requestGeneration = updateGeneration.current;
    pinApi
      .update(label, next)
      .then((state) => {
        // 应答只有可变字段，内容字段来自手里那份 payload。
        const merged = mergePinState(confirmedPinRef.current ?? pinRef.current, state);
        if (!merged) return;
        if (requestGeneration >= confirmedGeneration.current) {
          confirmedGeneration.current = requestGeneration;
          confirmedPinRef.current = merged;
        }
        // 用户在请求返回前继续调整时，旧响应不能覆盖本地乐观状态。
        if (!shouldApplyPinUpdateResponse(requestGeneration, updateGeneration.current, pendingUpdate.current)) {
          return;
        }
        pinRef.current = merged;
        setPin(merged);
      })
      .catch((reason) => {
        if (
          confirmedPinRef.current
          && shouldApplyPinUpdateResponse(
            requestGeneration,
            updateGeneration.current,
            pendingUpdate.current,
          )
        ) {
          pinRef.current = confirmedPinRef.current;
          setPin(confirmedPinRef.current);
        }
        setError(String(reason));
      })
      .finally(() => {
        updateInFlight.current = false;
        // 这一趟飞在天上时用户还在滚，落地后把攒下的补发出去。
        if (Object.keys(pendingUpdate.current).length > 0) scheduleFlush();
      });
  }, [label, scheduleFlush]);

  useEffect(() => {
    flushRef.current = flushUpdate;
  }, [flushUpdate]);

  const commitUpdate = useCallback(
    (update: PinUpdate) => {
      const normalized: PinUpdate = {
        ...update,
        ...(update.scale === undefined ? {} : { scale: Math.max(0.25, Math.min(4, update.scale)) }),
        ...(update.opacity === undefined ? {} : { opacity: Math.max(0.15, Math.min(1, update.opacity)) }),
      };
      setPin((current) => {
        const next = current ? { ...current, ...normalized } : current;
        pinRef.current = next;
        return next;
      });
      updateGeneration.current += 1;
      pendingUpdate.current = { ...pendingUpdate.current, ...normalized };
      scheduleFlush();
    },
    [scheduleFlush],
  );

  const adjustScale = useCallback(
    (delta: number) => {
      const current = pinRef.current;
      if (current) commitUpdate({ scale: current.scale + delta });
    },
    [commitUpdate],
  );

  const adjustOpacity = useCallback(
    (delta: number) => {
      const current = pinRef.current;
      if (current) commitUpdate({ opacity: current.opacity + delta });
    },
    [commitUpdate],
  );

  const runAction = useCallback((action: () => Promise<unknown>) => {
    setError(null);
    void action().catch((reason) => {
      console.error(reason);
      setError(t("pin.actionFailed"));
    });
  }, []);

  // 内容区的 CSS 尺寸与在窗口里的矩形。工具条按它选边，画布按它定尺寸。
  const mediaWidth = (pin?.contentWidth ?? 0) * (pin?.scale ?? 1);
  const mediaHeight = (pin?.contentHeight ?? 0) * (pin?.scale ?? 1);
  const mediaBox = {
    x: MEDIA_INSET,
    y: MEDIA_INSET,
    width: mediaWidth,
    height: mediaHeight,
  };

  const loadSourceImage = useCallback(() => pinApi.sourceImage(label), [label]);

  // 工具条能待的范围。**必须问后端**：窗口外框永远给工具条留够了位置，
  // 拿 viewport 当边界的话"超出屏幕自动调整"一次都不会触发（见 `usePinToolbarBounds`）。
  const toolbarBounds = usePinToolbarBounds(label, viewport, pinApi.toolbarBounds);

  const canvas = usePinCanvas({
    cssWidth: mediaWidth,
    cssHeight: mediaHeight,
    open: canvasOpen,
    initialProject: pin?.initialProject ?? null,
    loadSourceImage: loadSourceImage,
  });

  const copy = useCallback(async () => {
    setError(null);
    try {
      if (canvas.pristineProject) await pinApi.copy(label);
      else if (canvas.hasDocument && canvas.projectData) {
        await pinApi.copyCanvas(label, canvas.projectData);
      }
      else await pinApi.copy(label);
      setCopied(true);
      if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), 1000);
    } catch (reason) {
      console.error(reason);
      setError(t("pin.actionFailed"));
    }
  }, [canvas.hasDocument, canvas.pristineProject, canvas.projectData, label]);

  const showSaved = useCallback((path: string, messageKey = "pin.saved") => {
    setError(null);
    setNoticeKey(messageKey);
    setSavedPath(path);
    if (savedTimer.current !== null) window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSavedPath(null), 2200);
  }, []);

  /** 把画布上那一版存下来（顺带进剪贴板，"画完直接粘走"是最常见的下一步）。 */
  const saveCanvas = useCallback(async (mode: "editable" | "flat" = "editable") => {
    // 未修改工程复用 IDAT；真实编辑只提交文档，最终 PNG 由后端 renderer v2 生成。
    const project = canvas.pristineProject ? null : canvas.projectData;
    if (!canvas.pristineProject && !project) {
      throw new Error("Canvas source is unavailable");
    }
    const result = await pinApi.saveCanvas(
      label,
      null,
      mode === "editable",
      mode,
      project,
    );
    // 文件先落盘；即使随后剪贴板失败，这个 revision 也已经安全保存，不能诱导重复保存。
    if (mode === "editable") canvas.markSaved();
    showSaved(result.path, result.clipboardError ? "pin.savedClipboardFailed" : "pin.saved");
  }, [
    canvas.markSaved,
    canvas.pristineProject,
    canvas.projectData,
    label,
    showSaved,
  ]);

  const requestSave = useCallback(() => {
    if (canvas.hasDocument) setSavePrompt(true);
    else runAction(async () => showSaved(await pinApi.save(label)));
  }, [canvas.hasDocument, label, runAction, showSaved]);

  const patchWorkspace = useCallback((workspaceId: number | null, workspaceGroupId: number | null) => {
    setPin((value) => {
      const next = value ? { ...value, workspaceId, workspaceGroupId } : value;
      pinRef.current = next;
      return next;
    });
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    pinApi.onWorkspaceChanged((change) => {
      patchWorkspace(change.workspaceId, change.groupId);
      if (change.workspaceId == null) setWorkspaceOpen(false);
    }).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    }).catch((reason) => console.error(reason));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [patchWorkspace]);

  const loadWorkspaceGroups = useCallback(async () => {
    setWorkspaceBusy(true);
    setWorkspaceError(null);
    try {
      setWorkspaceGroups(await pinApi.groups());
    } catch (reason) {
      console.error(reason);
      setWorkspaceError(t("pin.workspaceLoadFailed"));
    } finally {
      setWorkspaceBusy(false);
    }
  }, []);

  const workspaceAction = useCallback(async (action: () => Promise<void>): Promise<boolean> => {
    setWorkspaceBusy(true);
    setWorkspaceError(null);
    try {
      await action();
      return true;
    } catch (reason) {
      console.error(reason);
      setWorkspaceError(t("pin.actionFailed"));
      return false;
    } finally {
      setWorkspaceBusy(false);
    }
  }, []);

  const useWorkspace = useCallback(() => {
    const current = pinRef.current;
    if (!current) return;
    if (current.workspaceId != null) {
      setOpacityOpen(false);
      setWorkspaceOpen(true);
      void loadWorkspaceGroups();
      return;
    }
    runAction(async () => {
      const project = canvas.hasDocument ? canvas.projectData : null;
      if (canvas.hasDocument && !project) throw new Error("Canvas source is unavailable");
      const status = await pinApi.saveWorkspace(label, project);
      patchWorkspace(status.workspaceId, status.groupId);
      if (canvas.hasDocument) canvas.markSaved();
    });
  }, [canvas.hasDocument, canvas.markSaved, canvas.projectData, label, loadWorkspaceGroups, patchWorkspace, runAction]);

  const assignWorkspaceGroup = useCallback((groupId: number | null) => {
    workspaceAction(async () => {
      await pinApi.assignGroup(label, groupId);
      patchWorkspace(pinRef.current?.workspaceId ?? null, groupId);
    });
  }, [label, patchWorkspace, workspaceAction]);

  const createWorkspaceGroup = useCallback((name: string) =>
    workspaceAction(async () => {
      const group = await pinApi.createGroup(name);
      await pinApi.assignGroup(label, group.id);
      setWorkspaceGroups((groups) => [...groups, group].sort((a, b) => a.sortOrder - b.sortOrder || a.id - b.id));
      patchWorkspace(pinRef.current?.workspaceId ?? null, group.id);
    }), [label, patchWorkspace, workspaceAction]);

  const renameWorkspaceGroup = useCallback((id: number, name: string) => {
    workspaceAction(async () => {
      if (!await pinApi.renameGroup(id, name)) throw new Error("Pin workspace group does not exist");
      setWorkspaceGroups((groups) => groups.map((group) => group.id === id ? { ...group, name } : group));
    });
  }, [workspaceAction]);

  const deleteWorkspaceGroup = useCallback((id: number) => {
    workspaceAction(async () => {
      if (!await pinApi.deleteGroup(id)) throw new Error("Pin workspace group does not exist");
      setWorkspaceGroups((groups) => groups.filter((group) => group.id !== id));
      if (pinRef.current?.workspaceGroupId === id) {
        patchWorkspace(pinRef.current.workspaceId, null);
      }
    });
  }, [patchWorkspace, workspaceAction]);

  const removeWorkspace = useCallback(() => {
    workspaceAction(async () => {
      await pinApi.removeWorkspace(label);
      patchWorkspace(null, null);
      setWorkspaceOpen(false);
    });
  }, [label, patchWorkspace, workspaceAction]);

  const finishClose = useCallback(async (save: boolean) => {
    if (closingRef.current) return;
    closingRef.current = true;
    setClosing(true);
    setError(null);
    try {
      const revision = canvas.currentRevision();
      if (save && pinRef.current?.workspaceId != null) {
        const project = canvas.projectData;
        if (!project) throw new Error("Canvas source is unavailable");
        await pinApi.saveWorkspace(label, project);
        canvas.markSaved();
      } else if (save) await saveCanvas("editable");
      // 除了禁用交互，再核对一次版本，防止已经排队的编辑越过保存边界。
      if (save && canvas.currentRevision() !== revision) return;
      await pinApi.close(label);
      setClosePrompt(false);
    } catch (reason) {
      console.error(reason);
      setError(t("pin.actionFailed"));
    } finally {
      closingRef.current = false;
      setClosing(false);
    }
  }, [canvas.currentRevision, canvas.markSaved, canvas.projectData, label, saveCanvas]);

  const saveFromDialog = useCallback(async (mode: "editable" | "flat") => {
    if (closingRef.current) return;
    closingRef.current = true;
    setClosing(true);
    setError(null);
    try {
      await saveCanvas(mode);
      setSavePrompt(false);
    } catch (reason) {
      console.error(reason);
      setError(t("pin.actionFailed"));
    } finally {
      closingRef.current = false;
      setClosing(false);
    }
  }, [saveCanvas]);

  /**
   * 关窗。画布上有没保存的东西就先问一句。
   *
   * 问的时机在这里而不是后端：只有前端知道画布是不是脏的（标注从不写回条目，
   * 见 `usePinCanvas`）。窗口的关闭按钮、Esc、右键菜单三条路都走这里。
   */
  const requestClose = useCallback(() => {
    if (closingRef.current || closePrompt || savePrompt) return;
    if (canvas.isDirty()) {
      setClosePrompt(true);
      return;
    }
    void finishClose(false);
  }, [canvas.isDirty, closePrompt, finishClose, savePrompt]);

  const closeHandler = useRef(requestClose);
  closeHandler.current = requestClose;
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void pinApi.onCloseRequested(() => closeHandler.current()).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    }).catch((reason) => {
      console.error(reason);
      setError(t("pin.actionFailed"));
    });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  function blockClosingInteraction(event: React.SyntheticEvent) {
    if (!closingRef.current && (!(closePrompt || savePrompt)
      || (event.target instanceof Element && event.target.closest("[data-pin-dialog]")))) return;
    event.preventDefault();
    event.stopPropagation();
  }

  useEffect(() => {
    function onPointerMove(event: PointerEvent) {
      // 画布开着的时候整块内容区都在画画，拖动窗口只剩把手与空白处那条路——
      // 否则画第一笔就把窗口拖走了。
      //
      // `isToolbarDragging()` 是第二道闸：工具条跟着指针走，指针很容易落到工具条外面，
      // 那一刻 `onControls` 为假、判据当场成立，"拖工具条"就变成"拖整个贴图窗口"。
      // pointer capture 已经让 target 钉在把手上，这条只防捕获没生效的环境。
      if (closingRef.current || closePrompt || savePrompt || pin?.locked || canvasOpen || isToolbarDragging()) {
        drag.current = NO_DRAG;
        return;
      }
      const next = trackDragMove(
        drag.current,
        {
          buttons: event.buttons,
          x: event.clientX,
          y: event.clientY,
          onControls: onPinControls(event.target),
        },
        performance.now(),
      );
      drag.current = next.state;
      if (next.start) services.startDragging().catch((reason) => setError(String(reason)));
    }
    window.addEventListener("pointermove", onPointerMove);
    return () => window.removeEventListener("pointermove", onPointerMove);
  }, [canvasOpen, closePrompt, savePrompt, pin?.locked, services]);

  /**
   * 滚轮、捏合、划选、拖拽这四件事都得用非被动的原生监听器接。
   *
   * React 的 `onWheel` 是被动监听器，`preventDefault()` 在那里无效；WebKitGTK 会把
   * 触控板捏合合成成 ctrl+滚轮，拦不住就变成页面缩放（内容溢出窗口、工具栏错位）。
   * `selectstart` / `dragstart` 同理：不拦住，一次拖动就把内容刷成系统强调色的选中高亮，
   * 看着就像"拖贴图变成了选中图片"。
   *
   * 注意这里**只**禁止缩放与划选：窗口照样能拖动（`pointerdown` + `startDragging`），
   * 也照样能点击获得焦点。
   */
  useEffect(() => {
    function onWheel(event: WheelEvent) {
      if (closePrompt || savePrompt) {
        // 普通滚轮交给弹窗正文；Ctrl/捏合仍不能缩放 WebKit 页面。
        const inBody = event.target instanceof Element && event.target.closest(".pin-dialog-body");
        if (event.ctrlKey || event.metaKey || !inBody) event.preventDefault();
        return;
      }
      event.preventDefault();
      if (closingRef.current) return;
      const intent = pinWheelIntent(event);
      if (intent.kind === "scale") adjustScale(intent.delta);
      else if (intent.kind === "opacity") adjustOpacity(intent.delta);
    }
    function onSelectStart(event: Event) {
      if (!allowsTextSelection(event.target)) event.preventDefault();
    }
    function onDragStart(event: Event) {
      event.preventDefault();
    }
    // Safari/WebKit 专有的捏合事件；类型定义里没有，但 WebKitGTK 会派发。
    function onGesture(event: Event) {
      event.preventDefault();
    }
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("selectstart", onSelectStart);
    window.addEventListener("dragstart", onDragStart);
    for (const name of ["gesturestart", "gesturechange", "gestureend"]) {
      window.addEventListener(name, onGesture, { passive: false });
    }
    return () => {
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("selectstart", onSelectStart);
      window.removeEventListener("dragstart", onDragStart);
      for (const name of ["gesturestart", "gesturechange", "gestureend"]) {
        window.removeEventListener(name, onGesture);
      }
    };
  }, [adjustOpacity, adjustScale, closePrompt, savePrompt]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!pin || event.defaultPrevented || event.isComposing) return;
      if (closingRef.current) { event.preventDefault(); return; }
      // 确认框拥有焦点与按键；不能在其后继续撤销、复制或缩放文档。
      if (closePrompt || savePrompt) {
        if (isZoomShortcut(event)) event.preventDefault();
        if (event.key === "Escape") {
          event.preventDefault();
          if (savePrompt) setSavePrompt(false);
          else setClosePrompt(false);
        }
        return;
      }
      // 原生输入语义优先，尤其是文字标注的复制、撤销以及 +/- 输入。
      if (event.target instanceof Element
        && event.target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
      const command = event.ctrlKey || event.metaKey;
      if (command && event.key.toLowerCase() === "c" && window.getSelection()?.toString()) return;
      // Ctrl/Cmd 加 +/-/0 是 WebKit 的页面缩放快捷键，和捏合一样要吃掉；
      // 顺手让它改贴图自己的缩放，这才是用户按这几个键想要的效果。
      if (isZoomShortcut(event)) {
        event.preventDefault();
        if (event.key === "+" || event.key === "=") adjustScale(0.1);
        else if (event.key === "-" || event.key === "_") adjustScale(-0.1);
        return;
      }
      // 画布开着时 Ctrl+Z/Y 归撤销栈；不开着则不拦，免得吃掉别的用途。
      if (canvasOpen && command && event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (event.shiftKey) canvas.redo();
        else canvas.undo();
        return;
      }
      // 确认框开着时 Esc = 取消。以前无条件走 requestClose()，而那时 `dirty` 仍为真，
      // 于是只是把 `closePrompt` 又设一次 true——框不关、也没别的反应，用户会觉得按键失灵。
      if (event.key === "Escape") {
        if (savePrompt) setSavePrompt(false);
        else if (closePrompt) setClosePrompt(false);
        else requestClose();
      }
      else if (command && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void copy();
      } else if (event.key === "+" || event.key === "=") adjustScale(0.1);
      else if (event.key === "-") adjustScale(-0.1);
      // 画布开着时这些字母键要留给以后的工具快捷键，而且用户可能在输入文字标注。
      else if (canvasOpen) return;
      else if (event.key.toLowerCase() === "l") commitUpdate({ locked: !pin.locked });
      else if (
        event.key.toLowerCase() === "t"
        && (pin.above || alwaysOnTop?.state !== "unsupported")
      ) {
        commitUpdate({ above: !pin.above });
      }
      else if (event.key.toLowerCase() === "s" && pin.canSave) requestSave();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
    // 依赖只收窄到真正用到的那两个回调（都来自 `useHistory` 的 `useCallback`，稳定），
    // 而不是整个 `canvas`：那个对象里有依赖 `annotations` 的字段，画一笔就变一次，
    // 于是每画一笔都要重挂一次 keydown 监听。
  }, [
    adjustScale,
    alwaysOnTop?.state,
    canvas.redo,
    canvas.undo,
    canvasOpen,
    closePrompt,
    commitUpdate,
    copy,
    label,
    pin,
    requestClose,
    requestSave,
    runAction,
    savePrompt,
  ]);

  useEffect(() => {
    return () => {
      if (wheelFrame.current !== null) cancelAnimationFrame(wheelFrame.current);
      if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
      if (remindTimer.current !== null) window.clearTimeout(remindTimer.current);
      if (savedTimer.current !== null) window.clearTimeout(savedTimer.current);
    };
  }, []);

  if (error && !pin) {
    return <div className="pin-error" role="alert">{error}</div>;
  }
  if (!pin) return null;

  // 未拿到能力前维持原行为；明确 unsupported 时隐藏“开启置顶”，但已开启的状态仍允许关闭。
  const aboveSupported = pin.above || alwaysOnTop?.state !== "unsupported";
  const aboveLimited =
    alwaysOnTop?.state === "degraded" || alwaysOnTop?.state === "permission_required";
  const aboveLabel = t(
    pin.above
      ? aboveLimited
        ? "pin.unpinAboveLimited"
        : "pin.unpinAbove"
      : aboveLimited
        ? "pin.pinAboveLimited"
        : "pin.pinAbove",
  );

  // `mediaWidth` / `mediaHeight` 在上面就算好了（画布与工具条都要用）。窗口外框可能
  // 比内容区大——矮贴图为了放下工具条有高度下限（`pin/window.rs::MIN_OUTER_HEIGHT`），
  // 多出来的高度必须留成透明留白，不能让图片在变高的框里居中，否则"贴回原处"当场就偏了。
  // `max-*` 而不是定死宽高：缩放时窗口尺寸落后本地状态一两帧，那几帧里内容区跟着窗口缩，
  // 才不会溢出被裁。
  const imageRendering = pixelSize
    ? pinImageRendering({
      cssWidth: mediaWidth,
      cssHeight: mediaHeight,
      pixelWidth: pixelSize.width,
      pixelHeight: pixelSize.height,
      deviceScale: pin.deviceScale,
      bufferScale: pin.bufferScale,
    })
    : "auto";

  const menuItems: PinMenuItem[] = [
    ...(aboveSupported
      ? [{
        id: "above",
        label: aboveLabel,
        checked: pin.above,
        onSelect: () => commitUpdate({ above: !pin.above }),
      }]
      : []),
    {
      id: "locked",
      label: t(pin.locked ? "pin.unlock" : "pin.lock"),
      checked: pin.locked,
      onSelect: () => commitUpdate({ locked: !pin.locked }),
    },
    {
      id: "workspace",
      label: t(pin.workspaceId == null ? "pin.workspaceSave" : "pin.workspaceManage"),
      checked: pin.workspaceId != null,
      onSelect: useWorkspace,
    },
    ...(pin.canSave
      ? [
        {
          id: "canvas",
          label: t(canvasOpen ? "pin.canvasClose" : "pin.canvasOpen"),
          checked: canvasOpen,
          onSelect: () => setCanvasOpen((open) => !open),
        },
        {
          id: "save",
          label: t("pin.save"),
          onSelect: requestSave,
        },
      ]
      : []),
    { id: "copy", label: t("pin.copy"), onSelect: () => void copy() },
    { id: "close", label: t("pin.close"), danger: true, onSelect: requestClose },
  ];

  return (
    <main
      className={`pin-root${pin.locked ? " locked" : ""}${canvasOpen ? " drawing" : ""}`}
      tabIndex={0}
      style={{ opacity: pin.opacity }}
      aria-busy={closing}
      onPointerDownCapture={blockClosingInteraction}
      onPointerMoveCapture={blockClosingInteraction}
      onPointerUpCapture={blockClosingInteraction}
      onKeyDownCapture={blockClosingInteraction}
      onClickCapture={blockClosingInteraction}
      onInputCapture={blockClosingInteraction}
      onChangeCapture={blockClosingInteraction}
      onPointerDown={(event) => {
        if (pin.locked || canvasOpen) return;
        drag.current = trackDragPointerDown(drag.current, {
          button: event.button,
          x: event.clientX,
          y: event.clientY,
          onControls: onPinControls(event.target),
        });
      }}
      onContextMenu={(event) => {
        // WebKit 自带的网页菜单已经在 GTK 层关掉了（`webview_hardening.rs`），
        // 这里把腾出来的右键接成快速操作。仍然 preventDefault：万一那一层没生效
        // （非 Linux、信号连接失败），也不要弹出"重新加载/检查元素"。
        event.preventDefault();
        // 同上：确认框开着时不叠一层菜单上去。
        if (closingRef.current || closePrompt || savePrompt) return;
        setMenuAt({ x: event.clientX, y: event.clientY });
      }}
    >
      <div className="pin-workspace" {...{ inert: closePrompt || savePrompt ? "" : undefined }} aria-hidden={closePrompt || savePrompt || undefined}>
      <section
        className={`pin-media ${pin.kind}${reminding ? " reminding" : ""}`}
        aria-label={t("pin.content")}
        style={
          pin.kind === "image"
            ? { maxWidth: `${mediaWidth}px`, maxHeight: `${mediaHeight}px` }
            : undefined
        }
      >
        {pin.kind === "color" ? (
          <div className="pin-color" data-color-pin>
            <div className="pin-color-swatch" aria-label={t("pin.colorSwatch")} style={{ backgroundColor: pin.color!.canonical }} />
            <dl className="pin-color-values">
              <div>
                <dt>{t("pin.colorOriginal")}</dt>
                <dd>{pin.text}</dd>
              </div>
              <div>
                <dt>{t("pin.colorCanonical")}</dt>
                <dd>{pin.color!.canonical}</dd>
              </div>
            </dl>
          </div>
        ) : pin.kind === "image" && imageUrl ? (
          <img
            src={imageUrl}
            alt={t("pin.imageAlt")}
            draggable={false}
            // 见 `rendering.ts`：屏上一个图片像素正好一个设备像素时，最近邻反而是最清晰的。
            style={{ imageRendering, visibility: canvas.visible ? "hidden" : "visible" }}
            onLoad={(event) => {
              const image = event.currentTarget;
              imageElement.current = image;
              setPixelSize({ width: image.naturalWidth, height: image.naturalHeight });
              setReady(true);
            }}
            onError={() => {
              setError(t("pin.imageLoadFailed"));
              setReady(true);
            }}
          />
        ) : (
          <pre>{pin.text || ""}</pre>
        )}
        {/* 有文档时关闭工具仍保留合成预览；只有打开工具时接收指针事件。 */}
        {canvas.visible && (
          <canvas
            ref={canvas.canvasRef}
            className={`pin-canvas${canvasOpen ? " editing" : ""}`}
            style={{ width: mediaWidth, height: mediaHeight }}
            onPointerDown={canvas.onPointerDown}
            onPointerMove={canvas.onPointerMove}
            onPointerUp={canvas.onPointerUp}
          />
        )}
      </section>
      <PinToolbar
        media={mediaBox}
        bounds={toolbarBounds}
        scale={pin.scale}
        opacity={pin.opacity}
        locked={pin.locked}
        above={pin.above}
        aboveSupported={aboveSupported}
        aboveLimited={aboveLimited}
        canvasOpen={canvasOpen}
        canSave={pin.canSave}
        workspaceSaved={pin.workspaceId != null}
        copied={copied}
        opacityOpen={opacityOpen}
        onScale={(scale) => commitUpdate({ scale })}
        onOpacity={(opacity) => commitUpdate({ opacity })}
        onToggleOpacity={() => setOpacityOpen((open) => !open)}
        onToggleLock={() => commitUpdate({ locked: !pin.locked })}
        onToggleAbove={() => commitUpdate({ above: !pin.above })}
        onToggleCanvas={() => setCanvasOpen((open) => !open)}
        onCopy={() => void copy()}
        onSave={() => {
          requestSave();
        }}
        onWorkspace={useWorkspace}
        onClose={requestClose}
      />
      {workspaceOpen && pin.workspaceId != null && (
        <PinWorkspacePopover
          groups={workspaceGroups}
          groupId={pin.workspaceGroupId}
          busy={workspaceBusy}
          error={workspaceError}
          onAssign={assignWorkspaceGroup}
          onCreate={createWorkspaceGroup}
          onRename={renameWorkspaceGroup}
          onDelete={deleteWorkspaceGroup}
          onRemove={removeWorkspace}
          onDismiss={() => setWorkspaceOpen(false)}
        />
      )}
      {canvasOpen && (
        <PinCanvasToolbar
          media={mediaBox}
          bounds={toolbarBounds}
          tool={canvas.tool}
          color={canvas.color}
          stroke={canvas.stroke}
          text={canvas.text}
          canUndo={canvas.canUndo}
          canRedo={canvas.canRedo}
          hasSelectedObject={canvas.hasSelectedObject}
          onTool={canvas.setTool}
          onColor={canvas.setColor}
          onStroke={canvas.setStroke}
          onText={canvas.setText}
          onUndo={canvas.undo}
          onRedo={canvas.redo}
          onDeleteObject={canvas.deleteSelected}
          onClose={() => setCanvasOpen(false)}
        />
      )}
      {menuAt && (
        <PinContextMenu at={menuAt} items={menuItems} onDismiss={() => setMenuAt(null)} />
      )}
      </div>
      {closePrompt && <PinSaveDialog mode="close" busy={closing} error={error}
        onSave={() => void finishClose(true)} onCancel={() => setClosePrompt(false)}
        onAlternative={() => void finishClose(false)} />}
      {savePrompt && !closePrompt && <PinSaveDialog mode="save" busy={closing} error={error}
        onSave={() => void saveFromDialog("editable")}
        onCancel={() => setSavePrompt(false)}
        onAlternative={() => void saveFromDialog("flat")} />}
      {/* 只有一个 toast 位（`pin.css` 把它绝对定位在右下角），所以两条消息不能各渲染
          一个——那样会完全叠在一起。出错优先：它是用户需要处置的那一条。 */}
      {!closePrompt && !savePrompt && (error || savedPath) && (
        <div className="pin-toast" role="status">
          {error ? t("pin.actionFailed") : t(noticeKey)}
        </div>
      )}
    </main>
  );
}
