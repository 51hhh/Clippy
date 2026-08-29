/**
 * 翻译服务的前端元数据：显示名、内置默认端点/模型，以及每个服务需要哪些附加字段。
 *
 * 设置页、主窗口翻译面板和截图选区翻译共用这一份表，避免同一个服务在三处
 * 各写一遍标签和默认端点。默认端点必须与 Rust provider 里的 DEFAULT_ENDPOINT 保持一致。
 */

import type { TranslationProvider, TranslationServiceConfig } from "./ipc-types.ts";

export interface TranslationProviderMeta {
  /** 服务名的 i18n key */
  nameKey: string;
  /** 后端内置默认端点，空配置时作为占位符展示 */
  defaultEndpoint: string;
  /** 后端内置默认模型，仅 OpenAI 兼容服务有意义 */
  defaultModel: string;
  /** 是否需要模型字段 */
  needsModel?: boolean;
  /** 是否需要 Azure 资源区域 */
  needsRegion?: boolean;
  /** 是否需要 GCP 项目 ID */
  needsProject?: boolean;
  /** 是否需要第二个凭据字段（有道 appSecret） */
  needsSecret?: boolean;
  /** 未配置凭据时是否会走非官方 web 端点 */
  hasWebFallback?: boolean;
}

export const TRANSLATION_PROVIDERS: Record<TranslationProvider, TranslationProviderMeta> = {
  libretranslate: {
    nameKey: "settings.translation.providerLibre",
    defaultEndpoint: "https://libretranslate.com",
    defaultModel: "",
  },
  openai_compatible: {
    nameKey: "settings.translation.providerOpenAI",
    defaultEndpoint: "https://api.openai.com/v1",
    defaultModel: "gpt-4o-mini",
    needsModel: true,
  },
  deepl: {
    nameKey: "settings.translation.providerDeepL",
    defaultEndpoint: "https://api-free.deepl.com",
    defaultModel: "",
    hasWebFallback: true,
  },
  google: {
    nameKey: "settings.translation.providerGoogle",
    defaultEndpoint: "https://translation.googleapis.com",
    defaultModel: "",
    needsProject: true,
    hasWebFallback: true,
  },
  bing: {
    nameKey: "settings.translation.providerBing",
    defaultEndpoint: "https://api.cognitive.microsofttranslator.com",
    defaultModel: "",
    needsRegion: true,
    hasWebFallback: true,
  },
  youdao: {
    nameKey: "settings.translation.providerYoudao",
    defaultEndpoint: "https://openapi.youdao.com",
    defaultModel: "",
    needsSecret: true,
    hasWebFallback: true,
  },
};

export const TRANSLATION_PROVIDER_IDS = Object.keys(
  TRANSLATION_PROVIDERS,
) as TranslationProvider[];

export const DEFAULT_TRANSLATION_PROVIDER: TranslationProvider = "libretranslate";

/** 未知 provider 名（比如更旧或更新版本写入的配置）统一回落到默认服务 */
export function normalizeTranslationProvider(value: unknown): TranslationProvider {
  return typeof value === "string" && value in TRANSLATION_PROVIDERS
    ? (value as TranslationProvider)
    : DEFAULT_TRANSLATION_PROVIDER;
}

export function translationProviderMeta(provider: unknown): TranslationProviderMeta {
  return TRANSLATION_PROVIDERS[normalizeTranslationProvider(provider)];
}

/**
 * 当前启用的第一个服务，与后端 `primary_service` 取值一致。
 * 截图选区浮层只有一张卡的位置，仍然只用这个服务。
 */
export function primaryTranslationService(
  services: TranslationServiceConfig[] | null | undefined,
): TranslationServiceConfig | null {
  return services?.find((service) => service.enabled) ?? null;
}

/**
 * 所有启用的服务，顺序与配置一致，与后端 `selected_services` 的参与集合相同。
 * 认不出的 provider 名（更新版本写入的服务）跳过，界面不为它留卡位。
 */
export function enabledTranslationServices(
  services: TranslationServiceConfig[] | null | undefined,
): TranslationServiceConfig[] {
  return (services ?? []).filter(
    (service) => service.enabled && service.provider in TRANSLATION_PROVIDERS,
  );
}
