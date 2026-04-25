/**
 * telemetry.js — 轻量可观测埋点
 *
 * 默认 noop：不记录、不缓存、不订阅，零开销。
 * 调用 enable(opts) 后才开始保留事件并通知订阅者。
 * 测试和未来诊断面板都通过 subscribe() 拉取数据。
 */

let _enabled = false;
let _bufferLimit = 0;
const _buffer = [];
const _subscribers = new Set();

/** 启用埋点。bufferLimit > 0 时保留最近 N 条用于回放。 */
export function enable({ bufferLimit = 100 } = {}) {
  _enabled = true;
  _bufferLimit = bufferLimit;
}

/** 关闭埋点并清空缓冲，主要用于测试隔离。 */
export function disable() {
  _enabled = false;
  _bufferLimit = 0;
  _buffer.length = 0;
  _subscribers.clear();
}

export function isEnabled() {
  return _enabled;
}

/** 上报事件。未启用时直接返回。 */
export function emit(event, payload) {
  if (!_enabled) return;
  const record = { event, payload, ts: Date.now() };
  if (_bufferLimit > 0) {
    _buffer.push(record);
    if (_buffer.length > _bufferLimit) {
      _buffer.shift();
    }
  }
  for (const fn of _subscribers) {
    try {
      fn(record);
    } catch (err) {
      console.warn("telemetry subscriber 抛出异常:", err);
    }
  }
}

/** 订阅事件，返回取消订阅函数。 */
export function subscribe(fn) {
  _subscribers.add(fn);
  return () => _subscribers.delete(fn);
}

/** 取出当前缓冲快照，按时间顺序。 */
export function snapshot() {
  return _buffer.slice();
}
