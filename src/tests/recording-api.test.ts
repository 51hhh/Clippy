import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, convertFileSrc } = vi.hoisted(() => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((path: string, protocol: string) => `${protocol}://localhost/${path}`),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc }));

import {
  getRecordingThumbnail,
  getRecordingMediaUrl,
  mergeRecordingSession,
  prepareRecordingPlayback,
  releaseRecordingPlayback,
} from "../js/api/recording.ts";

describe("recording playback API", () => {
  beforeEach(() => vi.clearAllMocks());

  it("accepts only the exact opaque playback lease contract", async () => {
    const lease = { token: "media-0000000000000001", mimeType: "video/webm" as const };
    invoke.mockResolvedValueOnce(lease);
    await expect(prepareRecordingPlayback("session-a", "final")).resolves.toEqual(lease);
    expect(invoke).toHaveBeenCalledWith("prepare_recording_playback", {
      sessionId: "session-a",
      artifactId: "final",
    });

    for (const invalid of [
      { token: "media-1", mimeType: "video/webm" },
      { token: "media-0000000000000001", mimeType: "video/mp4" },
      { token: "MEDIA-0000000000000001", mimeType: "video/webm" },
    ]) {
      invoke.mockResolvedValueOnce(invalid);
      await expect(prepareRecordingPlayback("session-a", "final"))
        .rejects.toThrow("invalid_playback_lease");
    }
  });

  it("builds media URLs from tokens and releases only that token", async () => {
    expect(getRecordingMediaUrl("media-0000000000000001"))
      .toBe("recording-media://localhost/media-0000000000000001");
    expect(convertFileSrc).toHaveBeenCalledWith("media-0000000000000001", "recording-media");
    expect(() => getRecordingMediaUrl("../recording.webm")).toThrow("invalid_playback_lease");

    invoke.mockResolvedValueOnce(undefined);
    await releaseRecordingPlayback("media-0000000000000001");
    expect(invoke).toHaveBeenCalledWith("release_recording_playback", {
      token: "media-0000000000000001",
    });
  });

  it("submits only the opaque session id for recovery merge", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await mergeRecordingSession("session-a");
    expect(invoke).toHaveBeenCalledWith("merge_recording_session", {
      sessionId: "session-a",
    });
  });

  it("accepts a bounded base64 thumbnail and rejects malformed payloads", async () => {
    invoke.mockResolvedValueOnce("cG5n");
    await expect(getRecordingThumbnail("session-a")).resolves.toBe("cG5n");
    expect(invoke).toHaveBeenCalledWith("get_recording_thumbnail", {
      sessionId: "session-a",
    });

    invoke.mockResolvedValueOnce("not base64!");
    await expect(getRecordingThumbnail("session-a")).rejects.toThrow("invalid_thumbnail");
  });
});
