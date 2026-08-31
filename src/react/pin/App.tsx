import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindowLabel, startDraggingCurrentWindow } from "../../js/api.ts";
import { pngBase64ToObjectUrl } from "../annotation/pngPipeline";
import { pinApi } from "./api";
import { allowsTextSelection, isZoomShortcut, pinWheelIntent } from "./gestures";
import { PinToolbar } from "./PinToolbar";
import type { PinPayload, PinUpdate } from "./types";
import { mergePinState, shouldApplyPinUpdateResponse } from "./update-order";
import { t } from "../shared/i18n";

const DRAG_THRESHOLD = 5;

export function App() {
  const label = getCurrentWindowLabel();
  const dragStart = useRef<{ x: number; y: number } | null>(null);
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
      if (wheelFrame.current !== null) return;
      wheelFrame.current = requestAnimationFrame(() => {
        wheelFrame.current = null;
        const next = pendingUpdate.current;
        pendingUpdate.current = {};
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
          });
      });
    },
    [label],
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
      if (!dragStart.current || pin?.locked) return;
      const distance = Math.hypot(event.clientX - dragStart.current.x, event.clientY - dragStart.current.y);
      if (distance < DRAG_THRESHOLD) return;
      dragStart.current = null;
      startDraggingCurrentWindow().catch((reason) => setError(String(reason)));
    }
    function clearDrag() {
      dragStart.current = null;
    }
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", clearDrag);
    window.addEventListener("pointercancel", clearDrag);
    return () => {
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", clearDrag);
      window.removeEventListener("pointercancel", clearDrag);
    };
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

  return (
    <main
      className={`pin-root${pin.locked ? " locked" : ""}`}
      tabIndex={0}
      style={{ opacity: pin.opacity }}
      onPointerDown={(event) => {
        if (event.button === 0 && !pin.locked && !(event.target as Element).closest("[data-pin-controls]")) {
          dragStart.current = { x: event.clientX, y: event.clientY };
        }
      }}
    >
      <section className={`pin-media ${pin.kind}`} aria-label={t("pin.content")}>
        {pin.kind === "image" && imageUrl ? (
          <img
            src={imageUrl}
            alt={t("pin.imageAlt")}
            draggable={false}
            onLoad={() => setReady(true)}
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
        canSave={pin.canSave}
        copied={copied}
        opacityOpen={opacityOpen}
        onScale={(scale) => commitUpdate({ scale })}
        onOpacity={(opacity) => commitUpdate({ opacity })}
        onToggleOpacity={() => setOpacityOpen((open) => !open)}
        onToggleLock={() => commitUpdate({ locked: !pin.locked })}
        onCopy={() => void copy()}
        onSave={() => runAction(() => pinApi.save(label))}
        onClose={() => runAction(() => pinApi.close(label))}
      />
      {error && <div className="pin-toast" role="status">{t("pin.actionFailed")}</div>}
    </main>
  );
}
