import { beforeEach, describe, expect, it, vi } from "vitest";

const { invoke, convertFileSrc } = vi.hoisted(() => ({
  invoke: vi.fn(),
  convertFileSrc: vi.fn((path: string, protocol: string) => `${protocol}://localhost/${path}`),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc }));

import {
  getRecordingStartCapabilities,
  getRecordingThumbnail,
  getRecordingMediaUrl,
  mergeRecordingSession,
  pollRecordingHealth,
  prepareRecordingPlayback,
  releaseRecordingPlayback,
  startCaptureRecording,
} from "../js/api/recording.ts";

describe("recording playback API", () => {
  beforeEach(() => vi.clearAllMocks());

  const catalog = {
    catalogId: "audio-catalog-0000000000000001",
    systemAudioDevices: [{
      id: "audio-device-0000000000000001-00",
      label: "Speakers",
      isDefault: true,
    }],
    microphoneDevices: [{
      id: "audio-device-0000000000000001-01",
      label: "USB microphone",
      isDefault: false,
    }],
  };

  it("validates start capabilities and submits only opaque device tokens", async () => {
    const capabilities = {
      audioModes: ["none", "systemAudio", "microphone", "systemAndMicrophone"],
      deviceCatalog: catalog,
      deviceEnumerationFailed: false,
    };
    invoke.mockResolvedValueOnce(capabilities);
    await expect(getRecordingStartCapabilities()).resolves.toEqual(capabilities);
    expect(invoke).toHaveBeenCalledWith("get_recording_start_capabilities");

    const selection = {
      sessionId: "session-a",
      monitorId: 0,
      x: 1,
      y: 2,
      width: 30,
      height: 40,
    };
    invoke.mockResolvedValueOnce(undefined);
    const systemAudio = {
      mode: "systemAudio" as const,
      catalogId: catalog.catalogId,
      systemDeviceId: catalog.systemAudioDevices[0].id,
      microphoneDeviceId: null,
    };
    await startCaptureRecording(selection, systemAudio);
    expect(invoke).toHaveBeenCalledWith("start_capture_recording", {
      selection,
      audioSelection: systemAudio,
    });

    invoke.mockResolvedValueOnce(undefined);
    const mixed = {
      mode: "systemAndMicrophone" as const,
      catalogId: catalog.catalogId,
      systemDeviceId: catalog.systemAudioDevices[0].id,
      microphoneDeviceId: catalog.microphoneDevices[0].id,
    };
    await startCaptureRecording(selection, mixed);
    expect(invoke).toHaveBeenCalledWith("start_capture_recording", {
      selection,
      audioSelection: mixed,
    });
  });

  it("rejects malformed start capabilities and client-side audio mode injection", async () => {
    for (const invalid of [
      null,
      {},
      { audioModes: [] },
      { audioModes: ["systemAudio", "none"] },
      { audioModes: ["none", "none"] },
      { audioModes: ["none", "camera"] },
      { audioModes: ["none"], deviceCatalog: catalog },
      {
        audioModes: ["none"],
        deviceCatalog: { ...catalog, catalogId: "native-id" },
        deviceEnumerationFailed: false,
      },
      {
        audioModes: ["none"],
        deviceCatalog: {
          ...catalog,
          microphoneDevices: [{ ...catalog.microphoneDevices[0], label: "bad\nlabel" }],
        },
        deviceEnumerationFailed: false,
      },
      {
        audioModes: ["none"],
        deviceCatalog: {
          ...catalog,
          microphoneDevices: [{ ...catalog.microphoneDevices[0], id: catalog.systemAudioDevices[0].id }],
        },
        deviceEnumerationFailed: false,
      },
    ]) {
      invoke.mockResolvedValueOnce(invalid);
      await expect(getRecordingStartCapabilities()).rejects.toThrow(
        "invalid_start_capabilities",
      );
    }

    await expect(startCaptureRecording({} as never, {
      mode: "camera",
      catalogId: null,
      systemDeviceId: null,
      microphoneDeviceId: null,
    } as never))
      .rejects.toThrow("invalid_audio_mode");
    await expect(startCaptureRecording({} as never, {
      mode: "microphone",
      catalogId: null,
      systemDeviceId: catalog.systemAudioDevices[0].id,
      microphoneDeviceId: null,
    })).rejects.toThrow("invalid_audio_mode");
  });

  it("polls recording worker health through the restricted control command", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await pollRecordingHealth();
    expect(invoke).toHaveBeenCalledWith("poll_recording_health");
  });

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
