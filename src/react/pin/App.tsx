import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindowLabel, startDraggingCurrentWindow } from "../../js/api.ts";
import { pngBase64ToObjectUrl } from "../annotation/pngPipeline";
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
import { pinImageRendering } from "./rendering";
import type { PinPayload, PinUpdate } from "./types";
import { mergePinState, shouldApplyPinUpdateResponse } from "./update-order";
import { usePinCanvas } from "./usePinCanvas";
import { t } from "../shared/i18n";

/** 事件落点是不是工具条/滑块那一片。 */
function onPinControls(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("[data-pin-controls]") !== null;
}

/** 内容区相对窗口原点的偏移，与 `pin.css` 的 `.pin-media` inset 和 `pin/window.rs` 的
 *  `SHADOW_GUTTER` 是一份契约。工具条要按内容区的位置选边，所以这里要知道它。 */
const MEDIA_INSET = 12;

export function App() {
  const label = getCurrentWindowLabel();
  const drag = useRef<DragTracking>(NO_DRAG);
  const wheelFrame = useRef<number | null>(null);
  const pendingUpdate = useRef<PinUpdate>({});
  const updateGeneration = useRef(0);
  const copiedTimer = useRef<number | null>(null);
  const pinRef = useRef<PinPayload | null>(null);
  const confirmedPinRef = useRef<PinPayload | null>(null);
  const confirmedGeneration = useRef(0);
  const [pin, setPin] = useState<PinPayload | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const [copied, setCopied] = useState(false);
  const [opacityOpen, setOpacityOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pixelSize, setPixelSize] = useState<{ width: number; height: number } | null>(null);
  const [reminding, setReminding] = useState(false);
  const remindTimer = useRef<number | null>(null);
  const [canvasOpen, setCanvasOpen] = useState(false);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  /** 关窗时"要不要保存画布"的询问。`null` 表示没在问。 */
  const [closePrompt, setClosePrompt] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const savedTimer = useRef<number | null>(null);
  const imageElement = useRef<HTMLImageElement | null>(null);
  const [viewport, setViewport] = useState({
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
    let cancelled = false;
    pinApi
      .get(label)
      .then((payload) => {
        if (!cancelled) {
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
    };
  }, [label]);

  useEffect(() => {
    pinRef.current = pin;
  }, [pin]);

  /**
   * 后台算好的清晰版图片换进来（见 `rendering.ts` 与 `pin/resample.rs`）。
   *
   * **只换 `imageBase64`**：这一刻用户可能已经滚过滚轮，整份覆盖会把缩放弹回去。
   * 三份引用都得更新——`flushUpdate` 是拿 `confirmedPinRef` 当基底去合并 `update_pin`
   * 的应答的，漏掉它的话下一次缩放就把原图换回来了。
   */
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    const sharpen = (current: PinPayload | null, imageBase64: string) =>
      current ? { ...current, imageBase64 } : current;
    pinApi
      .onSharpened((payload) => {
        if (payload.label !== label) return;
        pinRef.current = sharpen(pinRef.current, payload.imageBase64);
        confirmedPinRef.current = sharpen(confirmedPinRef.current, payload.imageBase64);
        setPin((current) => sharpen(current, payload.imageBase64));
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
  }, [label]);

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
        setReminding(false);
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
      setViewport({ width: window.innerWidth, height: window.innerHeight });
    }
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  useEffect(() => {
    if (!pin?.imageBase64) {
      setImageUrl(null);
      return;
    }
    const url = pngBase64ToObjectUrl(pin.imageBase64);
    setImageUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [pin?.imageBase64]);

  useEffect(() => {
    if (!pin || ready || (pin.kind === "image" && !imageUrl)) return;
    if (pin.kind === "text") {
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
   * **同一时刻只允许一个请求在飞。** `update_pin` 是同步命令，跑在 GTK 主线程上，
   * 里面还有改窗口尺寸和一次摆位的 D-Bus 往返；按 rAF 无条件发就是 60 次/秒地往
   * 主线程上压活，缩放于是一顿一顿的。改成"在飞就先攒着，落地了再发最新的一份"之后，
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

  const copy = useCallback(async () => {
    try {
      await pinApi.copy(label);
      setCopied(true);
      if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), 1000);
    } catch (reason) {
      console.error(reason);
      setError(t("pin.actionFailed"));
    }
  }, [label]);

  const runAction = useCallback((action: () => Promise<unknown>) => {
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

  const canvas = usePinCanvas({
    // 渲染期读 ref 本身不会触发重渲染，但 `onLoad` 里紧跟着有 `setPixelSize`，
    // 那次重渲染会把已加载的 <img> 带进来——所以画布不会停在"底图是 null"的状态。
    // 两件事必须留在同一个 onLoad 里，别把 setPixelSize 挪走。
    image: imageElement.current,
    pixelWidth: pixelSize?.width ?? 0,
    pixelHeight: pixelSize?.height ?? 0,
    cssWidth: mediaWidth,
    cssHeight: mediaHeight,
    open: canvasOpen,
  });

  const showSaved = useCallback((path: string) => {
    setSavedPath(path);
    if (savedTimer.current !== null) window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => setSavedPath(null), 2200);
  }, []);

  /** 把画布上那一版存下来（顺带进剪贴板，"画完直接粘走"是最常见的下一步）。 */
  const saveCanvas = useCallback(async () => {
    const pngBase64 = await canvas.exportPng();
    showSaved(await pinApi.saveCanvas(label, pngBase64, true));
  }, [canvas, label, showSaved]);

  /**
   * 关窗。画布上有没保存的东西就先问一句。
   *
   * 问的时机在这里而不是后端：只有前端知道画布是不是脏的（标注从不写回条目，
   * 见 `usePinCanvas`）。窗口的关闭按钮、Esc、右键菜单三条路都走这里。
   */
  const requestClose = useCallback(() => {
    if (canvas.dirty) {
      setClosePrompt(true);
      return;
    }
    runAction(() => pinApi.close(label));
  }, [canvas.dirty, label, runAction]);

  useEffect(() => {
    function onPointerMove(event: PointerEvent) {
      // 画布开着的时候整块内容区都在画画，拖动窗口只剩把手与空白处那条路——
      // 否则画第一笔就把窗口拖走了。
      if (pin?.locked || canvasOpen) {
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
      if (next.start) startDraggingCurrentWindow().catch((reason) => setError(String(reason)));
    }
    window.addEventListener("pointermove", onPointerMove);
    return () => window.removeEventListener("pointermove", onPointerMove);
  }, [canvasOpen, pin?.locked]);

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
      // 无条件 preventDefault：即使这一下什么都不做，也不能让它落到 WebKit 的页面缩放上。
      event.preventDefault();
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
  }, [adjustOpacity, adjustScale]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!pin) return;
      const command = event.ctrlKey || event.metaKey;
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
      if (event.key === "Escape") requestClose();
      else if (command && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void copy();
      } else if (event.key === "+" || event.key === "=") adjustScale(0.1);
      else if (event.key === "-") adjustScale(-0.1);
      // 画布开着时这些字母键要留给以后的工具快捷键，而且用户可能在输入文字标注。
      else if (canvasOpen) return;
      else if (event.key.toLowerCase() === "l") commitUpdate({ locked: !pin.locked });
      else if (event.key.toLowerCase() === "t") commitUpdate({ above: !pin.above });
      else if (event.key.toLowerCase() === "s" && pin.canSave) runAction(() => pinApi.save(label));
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [adjustScale, canvas, canvasOpen, commitUpdate, copy, label, pin, requestClose, runAction]);

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
    {
      id: "above",
      label: t(pin.above ? "pin.unpinAbove" : "pin.pinAbove"),
      checked: pin.above,
      onSelect: () => commitUpdate({ above: !pin.above }),
    },
    {
      id: "locked",
      label: t(pin.locked ? "pin.unlock" : "pin.lock"),
      checked: pin.locked,
      onSelect: () => commitUpdate({ locked: !pin.locked }),
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
          onSelect: () => {
            if (canvas.dirty) runAction(saveCanvas);
            else runAction(async () => showSaved(await pinApi.save(label)));
          },
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
        setMenuAt({ x: event.clientX, y: event.clientY });
      }}
    >
      <section
        className={`pin-media ${pin.kind}${reminding ? " reminding" : ""}`}
        aria-label={t("pin.content")}
        style={
          pin.kind === "image"
            ? { maxWidth: `${mediaWidth}px`, maxHeight: `${mediaHeight}px` }
            : undefined
        }
      >
        {pin.kind === "image" && imageUrl ? (
          <img
            src={imageUrl}
            alt={t("pin.imageAlt")}
            draggable={false}
            // 见 `rendering.ts`：屏上一个图片像素正好一个设备像素时，最近邻反而是最清晰的。
            style={{ imageRendering }}
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
        {/* 画布盖在图片上，尺寸与内容区一致。关着的时候整个不渲染，
            于是不开画布的贴图完全没有画布开销（这是绝大多数情况）。 */}
        {canvasOpen && (
          <canvas
            ref={canvas.canvasRef}
            className="pin-canvas"
            style={{ width: mediaWidth, height: mediaHeight }}
            onPointerDown={canvas.onPointerDown}
            onPointerMove={canvas.onPointerMove}
            onPointerUp={canvas.onPointerUp}
          />
        )}
      </section>
      <PinToolbar
        media={mediaBox}
        viewportWidth={viewport.width}
        viewportHeight={viewport.height}
        scale={pin.scale}
        opacity={pin.opacity}
        locked={pin.locked}
        above={pin.above}
        canvasOpen={canvasOpen}
        canSave={pin.canSave}
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
          // 画过东西就存画过的那一版，否则存原图。
          if (canvas.dirty) runAction(saveCanvas);
          else runAction(async () => showSaved(await pinApi.save(label)));
        }}
        onClose={requestClose}
      />
      {canvasOpen && (
        <PinCanvasToolbar
          media={mediaBox}
          viewportWidth={viewport.width}
          viewportHeight={viewport.height}
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
      {closePrompt && (
        <div className="pin-close-prompt" role="dialog" aria-label={t("pin.saveBeforeClose")}>
          <p>{t("pin.saveBeforeClose")}</p>
          <div className="pin-close-actions">
            <button
              type="button"
              className="primary"
              onClick={() => {
                setClosePrompt(false);
                runAction(async () => {
                  await saveCanvas();
                  await pinApi.close(label);
                });
              }}
            >
              {t("pin.saveAndClose")}
            </button>
            <button
              type="button"
              onClick={() => {
                setClosePrompt(false);
                runAction(() => pinApi.close(label));
              }}
            >
              {t("pin.discardAndClose")}
            </button>
            <button type="button" onClick={() => setClosePrompt(false)}>
              {t("pin.cancelClose")}
            </button>
          </div>
        </div>
      )}
      {savedPath && <div className="pin-toast" role="status">{t("pin.saved")}</div>}
      {error && <div className="pin-toast" role="status">{t("pin.actionFailed")}</div>}
    </main>
  );
}
