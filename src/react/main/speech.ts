import type { SpokenText } from "../../js/api.ts";

/** 音频播放器。测试注入替代实现，避免依赖 jsdom 里没有的 HTMLMediaElement 播放能力。 */
export type SpeechPlayer = {
  /** 播放完成后 resolve；播放失败 reject */
  play: (spoken: SpokenText) => Promise<void>;
  stop: () => void;
};

/** 同时只播一段：新的朗读请求或切换条目都会中断上一段。 */
let current: HTMLAudioElement | null = null;

function stopSpeech(): void {
  if (!current) return;
  current.pause();
  current = null;
}

function playSpokenText(spoken: SpokenText): Promise<void> {
  stopSpeech();
  // 音频由后端取回后以 data URL 播放，CSP 因此不必为第三方主机放开 media-src。
  const audio = new Audio(`data:${spoken.mime_type};base64,${spoken.audio_base64}`);
  current = audio;
  return new Promise((resolve, reject) => {
    const settle = (finish: () => void) => {
      if (current === audio) current = null;
      finish();
    };
    audio.onended = () => settle(resolve);
    audio.onerror = () => settle(() => reject(new Error("audio playback failed")));
    void audio.play().catch((error) => settle(() => reject(error)));
  });
}

export const audioElementPlayer: SpeechPlayer = { play: playSpokenText, stop: stopSpeech };
