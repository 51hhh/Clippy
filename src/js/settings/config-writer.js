/** 设置页的即时项与 Save 共用队列，每次写入都以最新后端配置为基底。 */
/**
 * @param {{
 *   getConfig: () => Promise<import('../ipc-types.ts').AppConfig>,
 *   updateConfig: (config: import('../ipc-types.ts').AppConfig, options?: unknown) => Promise<unknown>,
 *   onSaved?: (config: import('../ipc-types.ts').AppConfig) => void,
 * }} services
 */
export function createConfigWriter({ getConfig, updateConfig, onSaved = () => {} }) {
  let pending = Promise.resolve();
  function run(operation) {
    const result = pending.then(operation);
    pending = result.catch(() => {});
    return result;
  }
  return {
    run,
    write(patch, options) {
      return run(async () => {
        const next = { ...await getConfig(), ...patch };
        const outcome = await updateConfig(next, options);
        onSaved(next);
        return outcome;
      });
    },
  };
}
