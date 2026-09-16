import { useEffect, useRef, useState } from "react";
import type { StructuredOcr, TranslationBatch, ViewerColor, ViewerPayload, ViewerReply, ViewerRequest, ViewerTranslateOptions } from "../../js/ipc-types";
import type { ImageCodeScanResponse } from "../../js/api";
import type { ViewerServices } from "./api";
import { ViewerRequests } from "./requests";

export type TaskState<T> = { status: "idle" | "loading" | "ready" | "error"; value: T | null; error: string | null };
const idle = <T,>(): TaskState<T> => ({ status: "idle", value: null, error: null });
export function viewerError(reason: unknown): string {
  if (reason && typeof reason === "object" && "code" in reason) return String(reason.code);
  return "worker_failed";
}

export function useViewerTools(payload: ViewerPayload, services: ViewerServices) {
  const authority = useRef(new ViewerRequests(payload.handle)).current;
  const [ocr, setOcr] = useState<TaskState<StructuredOcr>>(idle);
  const [scan, setScan] = useState<TaskState<ImageCodeScanResponse>>(idle);
  const [translation, setTranslation] = useState<TaskState<TranslationBatch>>(idle);
  const [color, setColor] = useState<TaskState<ViewerColor>>(idle);
  const [sensitive, setSensitive] = useState(payload.source.sensitive);
  const inFlight = useRef(new Set<string>());
  useEffect(() => { authority.activate(); return () => { authority.dispose(); inFlight.current.clear(); }; }, [authority]);
  async function run<T>(channel: string, setter: (state: TaskState<T>) => void, execute: (request: ViewerRequest) => Promise<ViewerReply<T>>) {
    if (channel !== "color" && inFlight.current.has(channel)) return;
    const request = authority.begin(channel); inFlight.current.add(channel);
    setter({ status: "loading", value: null, error: null });
    try {
      const reply = await execute(request);
      if (authority.accepts(channel, request, reply)) setter({ status: "ready", value: reply.value, error: null });
    } catch (reason) {
      if (authority.accepts(channel, request)) {
        const code = viewerError(reason);
        if (code === "sensitive_content") setSensitive(true);
        setter({ status: "error", value: null, error: code });
      }
    } finally { if (authority.accepts(channel, request)) inFlight.current.delete(channel); }
  }
  return {
    authority, ocr, scan, translation, color, sensitive,
    recognize: () => run("ocr", setOcr, request => services.recognize(request)),
    detect: () => run("scan", setScan, request => services.scan(request)),
    translate: (options: ViewerTranslateOptions) => { if (!sensitive) return run("translation", (state: TaskState<TranslationBatch>) => {
      if (state.value?.services.some(result => result.status === "error" && result.code === "sensitive_content")) setSensitive(true);
      setTranslation(state);
    }, request => services.translate(request, options)); },
    sample: (x: number, y: number) => run("color", setColor, request => services.sample(request, x, y)),
  };
}
