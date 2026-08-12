/** 一个只允许最新异步请求提交结果的加载器。 */
export type LatestCaptureLoadResult<T> =
  | { applied: true; value: T }
  | { applied: false };

export type LatestCaptureLoader<Args extends unknown[], T> = {
  load: (...args: Args) => Promise<LatestCaptureLoadResult<T>>;
  invalidate: () => void;
};

/** 记录当前编辑窗口仍负有清理责任的截图代次。 */
export function createCaptureGenerationTracker() {
  const generations = new Set<number>();
  return {
    track(generation: number): boolean {
      if (!Number.isSafeInteger(generation) || generation <= 0) return false;
      generations.add(generation);
      return true;
    },
    release(generation: number): void {
      generations.delete(generation);
    },
    pending(): number[] {
      return [...generations];
    },
  };
}

const STALE_RESULT: { applied: false } = { applied: false };

/**
 * 为会被事件重复触发的加载操作提供 latest-request 语义。
 * 失效或被新请求取代的请求会以 applied=false 完成，避免旧错误冒泡到调用方。
 */
export function createLatestCaptureLoader<Args extends unknown[], T>(
  request: (...args: Args) => Promise<T>,
): LatestCaptureLoader<Args, T> {
  let generation = 0;

  return {
    load(...args) {
      const requestGeneration = ++generation;
      let requestPromise: Promise<T>;
      try {
        requestPromise = request(...args);
      } catch (error) {
        requestPromise = Promise.reject(error);
      }

      return requestPromise.then(
        (value): LatestCaptureLoadResult<T> => {
          if (requestGeneration !== generation) return STALE_RESULT;
          return { applied: true, value };
        },
        (error: unknown) => {
          if (requestGeneration !== generation) return STALE_RESULT;
          return Promise.reject(error);
        },
      );
    },
    invalidate() {
      generation += 1;
    },
  };
}
