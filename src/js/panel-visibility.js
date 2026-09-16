/** 串行提交面板可见性，失败时恢复最后一次已提交状态。 */
export function createPanelVisibilityController({ initial = false, apply, persist }) {
  let desired = initial;
  let committed = initial;
  let requestId = 0;
  let transition = Promise.resolve();

  return {
    isVisible() {
      return desired;
    },

    request(next) {
      const currentRequest = ++requestId;
      desired = next;
      apply(next);

      const persistRequest = transition.then(() => persist(next), () => persist(next));
      transition = persistRequest.then(
        () => {
          committed = next;
          if (currentRequest !== requestId) return null;
          desired = next;
          apply(next);
          return next;
        },
        (error) => {
          if (currentRequest === requestId) {
            desired = committed;
            apply(committed);
          }
          throw error;
        },
      );
      return transition;
    },
  };
}
