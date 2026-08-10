import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { pinApi } from "./api";
import { PinToolbar } from "./PinToolbar";
import type { PinPayload, PinUpdate } from "./types";
import { shouldApplyPinUpdateResponse } from "./update-order";

const DRAG_THRESHOLD = 5;

function imageObjectUrl(base64: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
}

export function App() {
  const windowHandle = getCurrentWindow();
  const label = windowHandle.label;
  const dragStart = useRef<{ x: number; y: number } | null>(null);
  const wheelFrame = useRef<number | null>(null);
  const pendingUpdate = useRef<PinUpdate>({});
  const updateGeneration = useRef(0);
  const copiedTimer = useRef<number | null>(null);
  const pinRef = useRef<PinPayload | null>(null);
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
        if (!cancelled) setPin(payload);
      })
      .catch((reason) => {
        if (!cancelled) {
          setError(String(reason));
          void pinApi.ready(label);
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
    const url = imageObjectUrl(pin.imageBase64);
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
    pinApi.ready(label).catch((reason) => setError(String(reason)));
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
          .then((payload) => {
            // 用户在请求返回前继续调整时，旧响应不能覆盖本地乐观状态。
            if (!shouldApplyPinUpdateResponse(requestGeneration, updateGeneration.current, pendingUpdate.current)) {
              return;
            }
            pinRef.current = payload;
            setPin(payload);
          })
          .catch((reason) => setError(String(reason)));
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
    await pinApi.copy(label);
    setCopied(true);
    if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
    copiedTimer.current = window.setTimeout(() => setCopied(false), 1000);
  }, [label]);

  useEffect(() => {
    function onPointerMove(event: PointerEvent) {
      if (!dragStart.current || pin?.locked) return;
      const distance = Math.hypot(event.clientX - dragStart.current.x, event.clientY - dragStart.current.y);
      if (distance < DRAG_THRESHOLD) return;
      dragStart.current = null;
      windowHandle.startDragging().catch((reason) => setError(String(reason)));
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
  }, [pin?.locked, windowHandle]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (!pin) return;
      const command = event.ctrlKey || event.metaKey;
      if (event.key === "Escape") void pinApi.close(label);
      else if (command && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void copy();
      } else if (event.key === "+" || event.key === "=") adjustScale(0.1);
      else if (event.key === "-") adjustScale(-0.1);
      else if (event.key.toLowerCase() === "l") commitUpdate({ locked: !pin.locked });
      else if (event.key.toLowerCase() === "s" && pin.canSave) void pinApi.save(label);
      else if (event.key.toLowerCase() === "e" && pin.canEdit) void pinApi.edit(label);
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [adjustScale, commitUpdate, copy, label, pin]);

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
      onWheel={(event) => {
        event.preventDefault();
        if (event.ctrlKey || event.metaKey) {
          adjustOpacity(event.deltaY > 0 ? -0.05 : 0.05);
        } else {
          adjustScale(event.deltaY > 0 ? -0.05 : 0.05);
        }
      }}
    >
      <section className={`pin-media ${pin.kind}`} aria-label="Pinned content">
        {pin.kind === "image" && imageUrl ? (
          <img
            src={imageUrl}
            alt="Pinned image"
            draggable={false}
            onLoad={() => setReady(true)}
            onError={() => {
              setError("Failed to load pinned image");
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
        canEdit={pin.canEdit}
        copied={copied}
        opacityOpen={opacityOpen}
        onScale={(scale) => commitUpdate({ scale })}
        onOpacity={(opacity) => commitUpdate({ opacity })}
        onToggleOpacity={() => setOpacityOpen((open) => !open)}
        onToggleLock={() => commitUpdate({ locked: !pin.locked })}
        onCopy={() => void copy()}
        onSave={() => void pinApi.save(label)}
        onEdit={() => void pinApi.edit(label)}
        onClose={() => void pinApi.close(label)}
      />
      {error && <div className="pin-toast" role="status">Action failed</div>}
    </main>
  );
}
