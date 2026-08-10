import { useCallback, useState } from "react";

export type HistoryState<T> = {
  past: T[];
  present: T;
  future: T[];
};

const MAX_HISTORY = 100;

export function useHistory<T>(initial: T) {
  const [history, setHistory] = useState<HistoryState<T>>({ past: [], present: initial, future: [] });

  const commit = useCallback((update: T | ((current: T) => T)) => {
    setHistory((current) => {
      const next = typeof update === "function"
        ? (update as (value: T) => T)(current.present)
        : update;
      if (Object.is(next, current.present)) return current;
      return commitHistory(current, next);
    });
  }, []);

  const undo = useCallback(() => {
    setHistory((current) => {
      return undoHistory(current);
    });
  }, []);

  const redo = useCallback(() => {
    setHistory((current) => {
      return redoHistory(current);
    });
  }, []);

  const reset = useCallback((value: T) => {
    setHistory({ past: [], present: value, future: [] });
  }, []);

  return {
    value: history.present,
    canUndo: history.past.length > 0,
    canRedo: history.future.length > 0,
    commit,
    undo,
    redo,
    reset,
  };
}

export function commitHistory<T>(current: HistoryState<T>, next: T): HistoryState<T> {
  return {
    past: [...current.past, current.present].slice(-MAX_HISTORY),
    present: next,
    future: [],
  };
}

export function undoHistory<T>(current: HistoryState<T>): HistoryState<T> {
  const previous = current.past[current.past.length - 1];
  if (previous === undefined) return current;
  return {
    past: current.past.slice(0, -1),
    present: previous,
    future: [current.present, ...current.future],
  };
}

export function redoHistory<T>(current: HistoryState<T>): HistoryState<T> {
  const next = current.future[0];
  if (next === undefined) return current;
  return {
    past: [...current.past, current.present].slice(-MAX_HISTORY),
    present: next,
    future: current.future.slice(1),
  };
}
