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
import { PinToolbar } from "./PinToolbar";
import { pinImageRendering } from "./rendering";
import type { PinPayload, PinUpdate } from "./types";
import { mergePinState, shouldApplyPinUpdateResponse } from "./update-order";
import { t } from "../shared/i18n";

/** 事件落点是不是工具条/滑块那一片。 */
function onPinControls(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest("[data-pin-controls]") !== null;
}

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

  useEffect(() => {
    function onPointerMove(event: PointerEvent) {
      if (pin?.locked) {
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
  }, [pin?.locked]);

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
      if (event.key === "Escape") runAction(() => pinApi.close(label));
      else if (command && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void copy();
      } else if (event.key === "+" || event.key === "=") adjustScale(0.1);
      else if (event.key === "-") adjustScale(-0.1);
      else if (event.key.toLowerCase() === "l") commitUpdate({ locked: !pin.locked });
      else if (event.key.toLowerCase() === "t") commitUpdate({ above: !pin.above });
      else if (event.key.toLowerCase() === "s" && pin.canSave) runAction(() => pinApi.save(label));
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [adjustScale, commitUpdate, copy, label, pin, runAction]);

  useEffect(() => {
    return () => {
      if (wheelFrame.current !== null) cancelAnimationFrame(wheelFrame.current);
      if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
    };
  }, []);

  if (error && !pin) {
    return <div className="pin-error" role="alert">{error}</div>;
  }
  if (!pin) return null;

  // 内容区的 CSS 尺寸。窗口外框可能比它大——矮贴图为了放下工具条有高度下限
  // （`pin/window.rs::MIN_OUTER_HEIGHT`），多出来的高度必须留成透明留白，
  // 不能让图片在变高的框里居中，否则"贴回原处"当场就偏了。`max-*` 而不是定死宽高：
  // 缩放时窗口尺寸落后本地状态一两帧，那几帧里内容区跟着窗口缩，才不会溢出被裁。
  const mediaWidth = pin.contentWidth * pin.scale;
  const mediaHeight = pin.contentHeight * pin.scale;
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

  return (
    <main
      className={`pin-root${pin.locked ? " locked" : ""}`}
      tabIndex={0}
      style={{ opacity: pin.opacity }}
      onPointerDown={(event) => {
        if (pin.locked) return;
        drag.current = trackDragPointerDown(drag.current, {
          button: event.button,
          x: event.clientX,
          y: event.clientY,
          onControls: onPinControls(event.target),
        });
      }}
    >
      <section
        className={`pin-media ${pin.kind}`}
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
      </section>
      <PinToolbar
        scale={pin.scale}
        opacity={pin.opacity}
        locked={pin.locked}
        above={pin.above}
        canSave={pin.canSave}
        copied={copied}
        opacityOpen={opacityOpen}
        onScale={(scale) => commitUpdate({ scale })}
        onOpacity={(opacity) => commitUpdate({ opacity })}
        onToggleOpacity={() => setOpacityOpen((open) => !open)}
        onToggleLock={() => commitUpdate({ locked: !pin.locked })}
        onToggleAbove={() => commitUpdate({ above: !pin.above })}
        onCopy={() => void copy()}
        onSave={() => runAction(() => pinApi.save(label))}
        onClose={() => runAction(() => pinApi.close(label))}
      />
      {error && <div className="pin-toast" role="status">{t("pin.actionFailed")}</div>}
    </main>
  );
}
