import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject } from "react";
import {
  clearPendingCapture,
  getPendingCapture,
  onCurrentWindowCloseRequested,
} from "../../js/api.ts";
import { pngBase64ToObjectUrl } from "./pngPipeline";
import {
  createCaptureGenerationTracker,
  createLatestCaptureLoader,
} from "./pendingCaptureLoader";
import type { CapturedScreenshot } from "./types";

type Params = {
  canvasRef: MutableRefObject<HTMLCanvasElement | null>;
  imageRef: MutableRefObject<HTMLImageElement | null>;
  onImageReady: (image: HTMLImageElement) => void;
  onStatus: (status: string) => void;
};

function initialCaptureGeneration(): number | null {
  const raw = new URLSearchParams(window.location.search).get("generation");
  if (!raw) return null;
  const generation = Number(raw);
  return Number.isSafeInteger(generation) && generation > 0 ? generation : null;
}

export function usePendingCaptureImage({
  canvasRef,
  imageRef,
  onImageReady,
  onStatus,
}: Params) {
  const imageObjectUrlRef = useRef<string | null>(null);
  const onImageReadyRef = useRef(onImageReady);
  const onStatusRef = useRef(onStatus);
  const generationTracker = useMemo(() => createCaptureGenerationTracker(), []);
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

  const releaseImage = useCallback(() => {
    captureLoader.invalidate();
    setPendingCapture(null);
    setImageReady(false);
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
    generationTracker.pending().forEach((generation) => {
      void clearPendingCapture(generation)
        .then(() => generationTracker.release(generation))
        .catch(() => undefined);
    });
  }, [canvasRef, captureLoader, generationTracker, imageRef, releaseImageObjectUrl]);

  const loadPendingCapture = useCallback((generation: number) => {
    if (!generationTracker.track(generation)) {
      onStatusRef.current("Failed to load screenshot");
      return;
    }
    void captureLoader
      .load(generation)
      .then((result) => {
        if (!result.applied) return;
        setPendingCapture(result.value);
        onStatusRef.current("Ready");
        void clearPendingCapture(result.value.generation)
          .then(() => generationTracker.release(generation))
          .catch(() => undefined);
      })
      .catch((error: unknown) => {
        console.error(error);
        onStatusRef.current("Failed to load screenshot");
      });
  }, [captureLoader, generationTracker]);

  useEffect(() => {
    const generation = initialCaptureGeneration();
    if (generation) loadPendingCapture(generation);
  }, [loadPendingCapture]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onCurrentWindowCloseRequested(releaseImage)
      .then((cleanup) => {
        if (disposed) {
          cleanup();
          return;
        }
        unlisten = cleanup;
      })
      .catch((error) => {
        console.warn("Failed to subscribe to capture close requests", error);
      });
    return () => {
      disposed = true;
      captureLoader.invalidate();
      unlisten?.();
    };
  }, [captureLoader, releaseImage]);

  useEffect(() => {
    return () => {
      releaseImage();
    };
  }, [releaseImage]);

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

  return { imageReady, pendingCapture, releaseImage };
}
