import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import {
  clearPendingCapture,
  getPendingCapture,
  onCaptureLoaded,
} from "../../js/api.ts";
import { pngBase64ToObjectUrl } from "./pngPipeline";
import { createLatestCaptureLoader } from "./pendingCaptureLoader";
import type { CapturedScreenshot } from "./types";

type Params = {
  canvasRef: MutableRefObject<HTMLCanvasElement | null>;
  imageRef: MutableRefObject<HTMLImageElement | null>;
  onImageReady: (image: HTMLImageElement) => void;
  onStatus: (status: string) => void;
};

export function usePendingCaptureImage({
  canvasRef,
  imageRef,
  onImageReady,
  onStatus,
}: Params) {
  const imageObjectUrlRef = useRef<string | null>(null);
  const onImageReadyRef = useRef(onImageReady);
  const onStatusRef = useRef(onStatus);
  const pendingCaptureGenerationRef = useRef<number | null>(null);
  const captureLoader = useMemo(() => createLatestCaptureLoader(getPendingCapture), []);
  const [pendingCapture, setPendingCapture] = useState<CapturedScreenshot | null>(null);
  const [imageReady, setImageReady] = useState(false);
  onImageReadyRef.current = onImageReady;
  onStatusRef.current = onStatus;

  const releaseImageObjectUrl = useCallback(() => {
    if (imageObjectUrlRef.current) {
      URL.revokeObjectURL(imageObjectUrlRef.current);
      imageObjectUrlRef.current = null;
    }
  }, []);

  const loadPendingCapture = useCallback(() => {
    void captureLoader
      .load()
      .then((result) => {
        if (!result.applied) return;
        pendingCaptureGenerationRef.current = result.value.generation;
        setPendingCapture(result.value);
        onStatusRef.current("Ready");
      })
      .catch((error: unknown) => {
        console.error(error);
        onStatusRef.current("Failed to load screenshot");
      });
  }, [captureLoader]);

  useEffect(() => {
    loadPendingCapture();
    return () => captureLoader.invalidate();
  }, [captureLoader, loadPendingCapture]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onCaptureLoaded(() => loadPendingCapture())
      .then((cleanup) => {
        if (disposed) cleanup();
        else unlisten = cleanup;
      })
      .catch((error) => console.warn("Failed to subscribe to capture updates", error));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadPendingCapture]);

  useEffect(() => {
    return () => {
      const canvas = canvasRef.current;
      if (canvas) {
        canvas.width = 1;
        canvas.height = 1;
      }
      if (imageRef.current) {
        imageRef.current.src = "";
        imageRef.current = null;
      }
      releaseImageObjectUrl();
      const generation = pendingCaptureGenerationRef.current;
      void clearPendingCapture(generation ?? undefined).catch(() => undefined);
    };
  }, [canvasRef, imageRef, releaseImageObjectUrl]);

  useEffect(() => {
    if (!pendingCapture) return;
    setImageReady(false);
    if (imageRef.current) {
      imageRef.current.src = "";
      imageRef.current = null;
    }
    releaseImageObjectUrl();
    const image = new Image();
    let cancelled = false;
    let objectUrl: string;
    try {
      objectUrl = pngBase64ToObjectUrl(pendingCapture.pngBase64);
      imageObjectUrlRef.current = objectUrl;
    } catch (error) {
      console.error(error);
      onStatusRef.current("Failed to decode screenshot");
      setPendingCapture(null);
      return;
    }
    image.onload = () => {
      if (cancelled) return;
      imageRef.current = image;
      onImageReadyRef.current(image);
      setImageReady(true);
      setPendingCapture(null);
    };
    image.onerror = () => {
      if (!cancelled) {
        onStatusRef.current("Failed to decode screenshot");
        setPendingCapture(null);
      }
    };
    image.src = objectUrl;
    return () => {
      cancelled = true;
      image.onload = null;
      image.onerror = null;
      if (imageRef.current !== image) {
        image.src = "";
        if (imageObjectUrlRef.current === objectUrl) {
          releaseImageObjectUrl();
        } else {
          URL.revokeObjectURL(objectUrl);
        }
      }
    };
  }, [imageRef, pendingCapture, releaseImageObjectUrl]);

  return { imageReady, pendingCapture };
}
