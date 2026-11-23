import { useCallback, useMemo, useRef, useState } from "react";
import type { Annotation } from "../annotation/types";
import { useHistory } from "../annotation/useHistory";
import { DEFAULT_IMAGE_ADJUSTMENTS } from "../annotation/imageAdjustments";
import { parseInitialPinProject } from "../pin/projectSchema";
import type { PinCanvasProject, ViewerPayload } from "../../js/ipc-types";

/** 父组件必须以 snapshotId 为 key；文档不能通过替换 image src 复用历史。 */
export function useViewerDocument(payload: ViewerPayload) {
  const initial = useMemo(() => parseInitialPinProject(payload.initialProject), [payload.initialProject]);
  const history = useHistory<Annotation[]>(initial?.annotations || []);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [revision, setRevision] = useState(0), saved = useRef(0), version = useRef(0);
  const [savedRevision, setSavedRevision] = useState(0);
  const advance = useCallback(() => { version.current++; setRevision(version.current); }, []);
  const commit = useCallback((update: Annotation[] | ((items: Annotation[]) => Annotation[])) => {
    history.commit(update); advance();
  }, [history.commit, advance]);
  const project: PinCanvasProject = useMemo(() => ({ rendererVersion: 2,
    sourceWidth: payload.source.width, sourceHeight: payload.source.height,
    annotations: history.value, adjustments: initial?.adjustments || { ...DEFAULT_IMAGE_ADJUSTMENTS },
  }), [payload.source.width, payload.source.height, history.value, initial]);
  const latest = useRef(project); latest.current = project;
  return {
    annotations: history.value, adjustments: initial?.adjustments || DEFAULT_IMAGE_ADJUSTMENTS,
    selectedId, setSelectedId, commit, project,
    dirty: revision !== savedRevision,
    isDirty: () => version.current !== saved.current,
    version: () => version.current,
    capture: () => ({ project: latest.current, revision: version.current }),
    markSaved: (value: number) => { if (value === version.current) { saved.current = value; setSavedRevision(value); } },
    canUndo: history.canUndo, canRedo: history.canRedo,
    undo: () => { if (history.canUndo) { history.undo(); advance(); setSelectedId(null); } },
    redo: () => { if (history.canRedo) { history.redo(); advance(); setSelectedId(null); } },
    deleteSelected: () => { if (selectedId) { commit(items => items.filter(item => item.id !== selectedId)); setSelectedId(null); } },
  };
}
