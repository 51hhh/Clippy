import {
  copyText,
  translateClip,
  type AppConfig,
  type ClipItem,
} from "../../js/api.ts";

export type TranslationSnapshot = {
  clip: ClipItem | null;
  config: AppConfig | null;
  loading: boolean;
  translatedText: string;
  detectedLanguage: string | null;
  feedback: "idle" | "complete" | "copied" | "copy_failed" | "error";
  errorCode: string | null;
  revision: number;
};

const ERROR_CODES = new Set([
  "empty_input", "input_too_large", "sensitive_content", "missing_api_key",
  "keyring_unavailable", "clip_unavailable", "image_unavailable", "ocr_failed",
  "invalid_endpoint", "unsupported_provider", "timeout", "network", "http_status",
  "response_too_large", "invalid_response", "stale_request", "internal",
]);

export function stableTranslationErrorCode(reason: unknown): string | null {
  const message = reason instanceof Error ? reason.message : String(reason);
  const match = /^translation\.([a-z_]+):/.exec(message);
  return match && ERROR_CODES.has(match[1]) ? match[1] : null;
}

export class TranslationStore {
  private snapshot: TranslationSnapshot = {
    clip: null,
    config: null,
    loading: false,
    translatedText: "",
    detectedLanguage: null,
    feedback: "idle",
    errorCode: null,
    revision: 0,
  };
  private listeners = new Set<() => void>();
  private generation = 0;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): TranslationSnapshot => this.snapshot;

  private commit(update: Partial<TranslationSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...update, revision: this.snapshot.revision + 1 };
    this.listeners.forEach((listener) => listener());
  }

  setConfig(config: AppConfig): void {
    // 配置切换后，旧服务的在途响应不能覆盖新配置下的界面结果。
    this.generation += 1;
    this.commit({
      config,
      loading: false,
      translatedText: "",
      detectedLanguage: null,
      feedback: "idle",
      errorCode: null,
    });
  }

  setClip(clip: ClipItem | null): void {
    this.generation += 1;
    this.commit({
      clip,
      loading: false,
      translatedText: "",
      detectedLanguage: null,
      feedback: "idle",
      errorCode: null,
    });
  }

  clear(): void {
    this.setClip(null);
  }

  async translate(): Promise<void> {
    const clip = this.snapshot.clip;
    if (!clip || clip.is_sensitive || this.snapshot.loading) return;
    const requestGeneration = ++this.generation;
    this.commit({ loading: true, feedback: "idle", errorCode: null, translatedText: "" });
    try {
      const result = await translateClip(clip.id);
      if (requestGeneration !== this.generation || this.snapshot.clip?.id !== clip.id) return;
      if (!result.translated_text?.trim()) throw new Error("translation.invalid_response:");
      this.commit({
        loading: false,
        translatedText: result.translated_text,
        detectedLanguage: result.detected_source_language,
        feedback: "complete",
      });
    } catch (error) {
      if (requestGeneration !== this.generation || this.snapshot.clip?.id !== clip.id) return;
      this.commit({
        loading: false,
        feedback: "error",
        errorCode: stableTranslationErrorCode(error),
      });
    }
  }

  async copy(): Promise<void> {
    const translatedText = this.snapshot.translatedText;
    if (!translatedText) return;
    const requestGeneration = this.generation;
    try {
      await copyText(translatedText);
      if (
        requestGeneration !== this.generation
        || translatedText !== this.snapshot.translatedText
      ) return;
      this.commit({ feedback: "copied" });
    } catch {
      if (
        requestGeneration !== this.generation
        || translatedText !== this.snapshot.translatedText
      ) return;
      this.commit({ feedback: "copy_failed" });
    }
  }
}

export const translationStore = new TranslationStore();
