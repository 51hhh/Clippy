import {
  copyText,
  speakClip,
  speakText,
  translateClip,
  translationHistory,
  type AppConfig,
  type ClipItem,
  type ServiceTranslation,
  type SpokenText,
  type TranslationHistoryEntry,
  type TranslationProvider,
} from "../../js/api.ts";
import { enabledTranslationServices } from "../../js/translation-providers";
import { audioElementPlayer, type SpeechPlayer } from "./speech";

/** 单个服务的结果卡。失败同样是一张卡，用户可以只重试出错的那个服务。 */
export type TranslationCard = {
  provider: TranslationProvider;
  loading: boolean;
  translatedText: string;
  detectedLanguage: string | null;
  /** 后端实际使用的目标语言；自动换向后它可能与设置里的目标语言不同 */
  targetLanguage: string;
  errorCode: string | null;
  copyFeedback: "idle" | "copied" | "copy_failed";
  /** 译文来自已保存的记录而不是本次请求；界面必须区分，否则用户以为刚翻译过 */
  fromHistory: boolean;
};

/** 正在朗读的对象：条目原文或某个服务的译文 */
export type SpeechTarget = "source" | TranslationProvider;

export type TranslationSnapshot = {
  clip: ClipItem | null;
  config: AppConfig | null;
  loading: boolean;
  /** 正在朗读的对象；null 表示当前没有音频在播 */
  speaking: SpeechTarget | null;
  /** 朗读失败的原因，null 表示上一次朗读没有失败 */
  speechErrorCode: string | null;
  /** 顺序与配置里的服务顺序一致 */
  cards: TranslationCard[];
  /** 整批的汇总状态；单个服务的失败细节留在自己的卡上 */
  feedback: "idle" | "complete" | "partial" | "error";
  errorCode: string | null;
  revision: number;
};

const ERROR_CODES = new Set([
  "empty_input", "input_too_large", "sensitive_content", "missing_api_key",
  "incomplete_credentials", "keyring_unavailable", "clip_unavailable", "image_unavailable",
  "capture_unavailable", "ocr_failed", "invalid_endpoint", "unsupported_provider",
  "no_service_enabled", "timeout", "network", "http_status", "invalid_credentials",
  "rate_limited", "quota_exceeded", "response_too_large", "invalid_response",
  "provider_endpoint_broken", "stale_request", "internal",
]);

export function stableTranslationErrorCode(reason: unknown): string | null {
  const message = reason instanceof Error ? reason.message : String(reason);
  const match = /^translation\.([a-z_]+):/.exec(message);
  return match && ERROR_CODES.has(match[1]) ? match[1] : null;
}

/**
 * 卡片上的错误码。认不出的码（更新后的后端新增的）退化为 `internal` 而不是 null：
 * 界面靠 `errorCode` 判断这张卡是否失败，置空会让失败卡被当成成功。
 */
function cardErrorCode(code: string | null): string {
  return code && ERROR_CODES.has(code) ? code : "internal";
}

function pendingCard(provider: TranslationProvider): TranslationCard {
  return {
    provider,
    loading: true,
    translatedText: "",
    detectedLanguage: null,
    targetLanguage: "",
    errorCode: null,
    copyFeedback: "idle",
    fromHistory: false,
  };
}

function failedCard(provider: TranslationProvider, errorCode: string | null): TranslationCard {
  return { ...pendingCard(provider), loading: false, errorCode: cardErrorCode(errorCode) };
}

/** 服务返回空文本按无效响应处理，否则用户只会看到一张空白卡 */
function cardFromService(service: ServiceTranslation): TranslationCard {
  if (service.status === "error") return failedCard(service.provider, service.code);
  if (!service.translated_text?.trim()) return failedCard(service.provider, "invalid_response");
  return {
    provider: service.provider,
    loading: false,
    translatedText: service.translated_text,
    detectedLanguage: service.detected_source_language,
    targetLanguage: service.target_language ?? "",
    errorCode: null,
    copyFeedback: "idle",
    fromHistory: false,
  };
}

/** 已保存的记录。源语言记成 "auto" 表示服务当时没报告检测结果，不冒充检测语言。 */
function cardFromHistory(entry: TranslationHistoryEntry): TranslationCard {
  return {
    provider: entry.provider,
    loading: false,
    translatedText: entry.translated_text,
    detectedLanguage: entry.source_language === "auto" ? null : entry.source_language,
    targetLanguage: entry.target_language,
    errorCode: null,
    copyFeedback: "idle",
    fromHistory: true,
  };
}

/** 汇总所有卡的结果：全成功、部分失败还是全失败 */
function summarize(cards: TranslationCard[]): Pick<TranslationSnapshot, "feedback" | "errorCode"> {
  const failed = cards.filter((card) => card.errorCode);
  if (!cards.length) return { feedback: "idle", errorCode: null };
  if (!failed.length) return { feedback: "complete", errorCode: null };
  if (failed.length < cards.length) return { feedback: "partial", errorCode: null };
  // 全部失败且原因一致时才给出具体原因，否则汇总行只能是通用提示。
  const codes = new Set(failed.map((card) => card.errorCode));
  return { feedback: "error", errorCode: codes.size === 1 ? failed[0].errorCode : null };
}

export class TranslationStore {
  private snapshot: TranslationSnapshot = {
    clip: null,
    config: null,
    loading: false,
    speaking: null,
    speechErrorCode: null,
    cards: [],
    feedback: "idle",
    errorCode: null,
    revision: 0,
  };
  private listeners = new Set<() => void>();
  private generation = 0;

  /** 播放器可注入：测试不依赖 jsdom 里缺失的音频播放能力 */
  constructor(private readonly player: SpeechPlayer = audioElementPlayer) {}

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): TranslationSnapshot => this.snapshot;

  private commit(update: Partial<TranslationSnapshot>): void {
    this.snapshot = { ...this.snapshot, ...update, revision: this.snapshot.revision + 1 };
    this.listeners.forEach((listener) => listener());
  }

  /** 在途请求的结果是否还属于当前的条目与设置 */
  private isCurrent(generation: number, clip: ClipItem): boolean {
    return generation === this.generation && this.snapshot.clip?.id === clip.id;
  }

  /** 整批或任何一张卡在途时不接受新请求：后端按 request-id 淘汰旧请求，
   * 新请求会让在途批次整体作废。 */
  private isBusy(): boolean {
    return this.snapshot.loading || this.snapshot.cards.some((card) => card.loading);
  }

  setConfig(config: AppConfig): void {
    // 配置切换后，旧服务的在途响应不能覆盖新配置下的界面结果。
    this.generation += 1;
    this.commit({ config, ...this.reset() });
    // 服务列表可能刚变化，按新配置重新摆出已保存的译文。
    void this.loadHistory(this.snapshot.clip, this.generation);
  }

  setClip(clip: ClipItem | null): void {
    this.generation += 1;
    this.commit({ clip, ...this.reset() });
    void this.loadHistory(clip, this.generation);
  }

  /**
   * 把这条条目之前存下的译文先摆出来，用户不必为同一条目重复请求服务。
   * 只填当前启用的服务，且只在还没有卡片时填，避免覆盖在途请求或新结果。
   * 汇总行保持 idle：这不是本次翻译的结果，不该显示成"翻译完成"。
   */
  private async loadHistory(clip: ClipItem | null, generation: number): Promise<void> {
    if (!clip || clip.is_sensitive) return;
    let entries: TranslationHistoryEntry[];
    try {
      entries = await translationHistory(clip.id);
    } catch {
      // 历史是附带功能，读不到就当没有记录。
      return;
    }
    if (!this.isCurrent(generation, clip) || this.isBusy() || this.snapshot.cards.length) return;
    const cards = enabledTranslationServices(this.snapshot.config?.translation_services)
      // 记录按时间倒序返回，同一服务取到的就是最近一次译文。
      .map((service) => entries.find((entry) =>
        entry.provider === service.provider && entry.translated_text.trim()))
      .filter((entry): entry is TranslationHistoryEntry => Boolean(entry))
      .map(cardFromHistory);
    if (cards.length) this.commit({ cards });
  }

  private reset(): Partial<TranslationSnapshot> {
    // 条目或配置一变，正在播的音频就不再属于界面上的内容，立刻停掉。
    this.player.stop();
    return {
      loading: false,
      speaking: null,
      speechErrorCode: null,
      cards: [],
      feedback: "idle",
      errorCode: null,
    };
  }

  clear(): void {
    this.setClip(null);
  }

  /** 翻译当前条目：所有启用的服务并行执行，每个服务一张结果卡 */
  async translate(): Promise<void> {
    const clip = this.snapshot.clip;
    if (!clip || clip.is_sensitive || this.isBusy()) return;
    const requestGeneration = ++this.generation;
    // 先按配置顺序摆出占位卡，等待期间用户就知道有几个服务在跑。
    const pending = enabledTranslationServices(this.snapshot.config?.translation_services)
      .map((service) => pendingCard(service.provider));
    this.commit({ loading: true, feedback: "idle", errorCode: null, cards: pending });
    try {
      const batch = await translateClip(clip.id);
      if (!this.isCurrent(requestGeneration, clip)) return;
      const cards = batch.services.map(cardFromService);
      this.commit({ loading: false, cards, ...summarize(cards) });
    } catch (error) {
      if (!this.isCurrent(requestGeneration, clip)) return;
      // 请求级失败（没有启用服务、条目不可用等）没有单服务归属，清空卡片只留汇总。
      this.commit({
        loading: false,
        cards: [],
        feedback: "error",
        errorCode: stableTranslationErrorCode(error),
      });
    }
  }

  /** 只重试一个服务，其余卡的结果原样保留 */
  async retry(provider: TranslationProvider): Promise<void> {
    const clip = this.snapshot.clip;
    if (!clip || clip.is_sensitive || this.isBusy()) return;
    if (!this.snapshot.cards.some((card) => card.provider === provider)) return;
    const requestGeneration = ++this.generation;
    this.commit({ cards: this.replaceCard(pendingCard(provider)), feedback: "idle", errorCode: null });
    try {
      const batch = await translateClip(clip.id, [provider]);
      if (!this.isCurrent(requestGeneration, clip)) return;
      const service = batch.services.find((entry) => entry.provider === provider);
      this.applyCard(service ? cardFromService(service) : failedCard(provider, "invalid_response"));
    } catch (error) {
      if (!this.isCurrent(requestGeneration, clip)) return;
      // 重试只涉及这一个服务，失败原因就挂在它自己的卡上。
      this.applyCard(failedCard(provider, stableTranslationErrorCode(error)));
    }
  }

  private replaceCard(next: TranslationCard): TranslationCard[] {
    return this.snapshot.cards.map((card) => (card.provider === next.provider ? next : card));
  }

  private applyCard(next: TranslationCard): void {
    const cards = this.replaceCard(next);
    this.commit({ cards, ...summarize(cards) });
  }

  /** 朗读条目自身的文本。敏感条目在后端同样被拒绝，这里先不发请求 */
  async speakSource(): Promise<void> {
    const clip = this.snapshot.clip;
    if (!clip) return;
    await this.speak("source", () => speakClip(clip.id));
  }

  /** 朗读某个服务的译文，按它实际使用的目标语言发音 */
  async speakTranslation(provider: TranslationProvider): Promise<void> {
    const card = this.snapshot.cards.find((entry) => entry.provider === provider);
    if (!card?.translatedText) return;
    await this.speak(provider, () => speakText(card.translatedText, card.targetLanguage || undefined));
  }

  /**
   * 取音频再播放。一次只播一段：正在播时忽略新请求，
   * 否则用户连点两下会听到两段叠在一起的声音。
   */
  private async speak(target: SpeechTarget, request: () => Promise<SpokenText>): Promise<void> {
    const clip = this.snapshot.clip;
    if (!clip || clip.is_sensitive || this.snapshot.speaking) return;
    const requestGeneration = this.generation;
    this.commit({ speaking: target, speechErrorCode: null });
    try {
      const spoken = await request();
      // 条目或配置已经变了：不要在新界面上放旧内容的声音。
      if (!this.isCurrent(requestGeneration, clip)) return;
      await this.player.play(spoken);
      this.finishSpeech(target, null);
    } catch (error) {
      this.finishSpeech(target, cardErrorCode(stableTranslationErrorCode(error)));
    }
  }

  private finishSpeech(target: SpeechTarget, errorCode: string | null): void {
    if (this.snapshot.speaking !== target) return;
    this.commit({ speaking: null, speechErrorCode: errorCode });
  }

  /** 复制某个服务的译文；只有这张卡显示复制反馈 */
  async copy(provider: TranslationProvider): Promise<void> {
    const card = this.snapshot.cards.find((entry) => entry.provider === provider);
    const translatedText = card?.translatedText;
    if (!translatedText) return;
    const requestGeneration = this.generation;
    try {
      await copyText(translatedText);
      this.applyCopyFeedback(requestGeneration, provider, translatedText, "copied");
    } catch {
      this.applyCopyFeedback(requestGeneration, provider, translatedText, "copy_failed");
    }
  }

  private applyCopyFeedback(
    generation: number,
    provider: TranslationProvider,
    translatedText: string,
    copyFeedback: TranslationCard["copyFeedback"],
  ): void {
    if (generation !== this.generation) return;
    const card = this.snapshot.cards.find((entry) => entry.provider === provider);
    if (!card || card.translatedText !== translatedText) return;
    this.commit({
      cards: this.snapshot.cards.map((entry) => ({
        ...entry,
        copyFeedback: entry.provider === provider ? copyFeedback : "idle",
      })),
    });
  }
}

export const translationStore = new TranslationStore();
