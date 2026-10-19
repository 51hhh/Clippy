import { useEffect, useState } from "react";
import type { pinApi } from "./api";

/** Windows 原生 DPI 独立于 payload/乐观更新；未知时 NaN 让既有滤镜判据回退 auto。 */
export function usePinDisplayScale(
  enabled: boolean,
  api: Pick<typeof pinApi, "displayScale" | "onDisplayScaleChanged">,
  initialScale: number,
): number {
  const [scale, setScale] = useState(Number.NaN);
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    setScale(Number.NaN);
    function accept(value: number) {
      if (cancelled) return;
      revision++;
      setScale(typeof value === "number" && Number.isFinite(value) && value > 0 ? value : Number.NaN);
    }
    // 先挂监听，避免 DPI 在查询前后改变而丢失；任何更新都使迟到首读失效。
    void api.onDisplayScaleChanged(accept)
      .then(async (stop) => {
        if (cancelled) { stop(); return; }
        unlisten = stop;
        const readRevision = revision;
        try {
          const value = await api.displayScale();
          if (revision === readRevision) accept(value);
        } catch (reason) {
          if (revision === readRevision) accept(Number.NaN);
          console.debug("贴图实时 DPI 查询失败", reason);
        }
      })
      .catch((reason) => {
        accept(Number.NaN);
        console.debug("贴图实时 DPI 监听失败", reason);
      });
    return () => { cancelled = true; unlisten?.(); };
  }, [api, enabled]);
  return enabled ? scale : initialScale;
}
