/** 设置页的即时项与 Save 共用队列，每次写入都以最新后端配置为基底。 */
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
