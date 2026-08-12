/** 一个只允许最新异步请求提交结果的加载器。 */
export type LatestCaptureLoadResult<T> =
  | { applied: true; value: T }
  | { applied: false };

export type LatestCaptureLoader<T> = {
  load: () => Promise<LatestCaptureLoadResult<T>>;
  invalidate: () => void;
};

const STALE_RESULT: { applied: false } = { applied: false };

/**
 * 为会被事件重复触发的加载操作提供 latest-request 语义。
 * 失效或被新请求取代的请求会以 applied=false 完成，避免旧错误冒泡到调用方。
 */
export function createLatestCaptureLoader<T>(
  request: () => Promise<T>,
): LatestCaptureLoader<T> {
  let generation = 0;

  return {
    load() {
      const requestGeneration = ++generation;
      let requestPromise: Promise<T>;
      try {
        requestPromise = request();
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
