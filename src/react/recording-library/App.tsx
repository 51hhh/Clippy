import { useCallback, useEffect, useRef, useState } from "react";
import {
  closeRecordingLibrary,
  deleteRecordingSession,
  exportRecordingArtifact,
  listRecordings,
  recordingLibraryReady,
  revealRecordingArtifact,
  startRecordingLibraryDrag,
} from "../../js/api.ts";
import type { RecordingLibraryArtifact, RecordingLibraryItem } from "../../js/ipc-types.ts";
import { t } from "../../i18n/i18n.js";

export type RecordingLibraryServices = {
  ready: typeof recordingLibraryReady;
  list: typeof listRecordings;
  exportArtifact: typeof exportRecordingArtifact;
  revealArtifact: typeof revealRecordingArtifact;
  deleteSession: typeof deleteRecordingSession;
  startDrag: typeof startRecordingLibraryDrag;
  close: typeof closeRecordingLibrary;
};

const defaultServices: RecordingLibraryServices = {
  ready: recordingLibraryReady,
  list: listRecordings,
  exportArtifact: exportRecordingArtifact,
  revealArtifact: revealRecordingArtifact,
  deleteSession: deleteRecordingSession,
  startDrag: startRecordingLibraryDrag,
  close: closeRecordingLibrary,
};

export function formatRecordingDuration(milliseconds: number): string {
  const totalSeconds = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${hours}:${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`
    : `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export function formatRecordingBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function artifactBusyKey(sessionId: string, artifactId: string): string {
  return `${sessionId}:${artifactId}`;
}

function ArtifactRow({
  session,
  artifact,
  busyKey,
  onExport,
  onReveal,
}: {
  session: RecordingLibraryItem;
  artifact: RecordingLibraryArtifact;
  busyKey: string | null;
  onExport: (sessionId: string, artifactId: string) => void;
  onReveal: (sessionId: string, artifactId: string) => void;
}) {
  const key = artifactBusyKey(session.sessionId, artifact.artifactId);
  const busy = busyKey === key;
  return (
    <li className="recording-artifact">
      <div className="artifact-copy">
        <strong>{artifact.displayName}</strong>
        <span>{formatRecordingDuration(artifact.durationMs)} · {formatRecordingBytes(artifact.byteLength)}</span>
      </div>
      <div className="artifact-actions">
        <button type="button" disabled={busyKey !== null} onClick={() => onReveal(session.sessionId, artifact.artifactId)}>
          {t("recordings.showInFolder")}
        </button>
        <button className="primary" type="button" disabled={busyKey !== null} onClick={() => onExport(session.sessionId, artifact.artifactId)}>
          {busy ? t("recordings.working") : t("recordings.export")}
        </button>
      </div>
    </li>
  );
}

export function App({ services = defaultServices }: { services?: RecordingLibraryServices } = {}) {
  const generation = useRef(0);
  const [items, setItems] = useState<RecordingLibraryItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);

  const load = useCallback(async () => {
    const current = ++generation.current;
    setLoading(true);
    setError(false);
    try {
      const next = await services.list();
      if (current === generation.current) setItems(next);
    } catch {
      if (current === generation.current) setError(true);
    } finally {
      if (current === generation.current) setLoading(false);
    }
  }, [services]);

  useEffect(() => {
    void load().finally(() => services.ready().catch(() => undefined));
    return () => {
      generation.current += 1;
    };
  }, [load, services]);

  const runArtifact = async (
    sessionId: string,
    artifactId: string,
    action: (sessionId: string, artifactId: string) => Promise<unknown>,
  ) => {
    if (busyKey) return;
    const key = artifactBusyKey(sessionId, artifactId);
    setBusyKey(key);
    setError(false);
    try {
      await action(sessionId, artifactId);
    } catch {
      setError(true);
    } finally {
      setBusyKey(null);
    }
  };

  const remove = async (sessionId: string) => {
    if (busyKey) return;
    setBusyKey(`delete:${sessionId}`);
    setError(false);
    try {
      await services.deleteSession(sessionId);
      setConfirmDelete(null);
      await load();
    } catch {
      setError(true);
    } finally {
      setBusyKey(null);
    }
  };

  return (
    <main className="recordings-shell">
      <header className="recordings-titlebar" onMouseDown={(event) => {
        if (event.button === 0 && event.target === event.currentTarget) void services.startDrag();
      }}>
        <div>
          <h1>{t("recordings.title")}</h1>
          <p>{t("recordings.subtitle")}</p>
        </div>
        <button className="window-close" type="button" aria-label={t("recordings.close")} onClick={() => void services.close()}>×</button>
      </header>

      <section className="recordings-content" aria-live="polite">
        {loading && <div className="library-state">{t("recordings.loading")}</div>}
        {!loading && error && (
          <div className="library-state error-state" role="status">
            <p>{t("recordings.error")}</p>
            <button type="button" onClick={() => void load()}>{t("recordings.retry")}</button>
          </div>
        )}
        {!loading && !error && items.length === 0 && (
          <div className="library-state empty-state">
            <div className="empty-icon" aria-hidden="true">◉</div>
            <h2>{t("recordings.emptyTitle")}</h2>
            <p>{t("recordings.emptyBody")}</p>
          </div>
        )}
        {!loading && items.map((item) => (
          <article className="recording-card" key={item.sessionId}>
            <div className="recording-summary">
              <div>
                <div className="recording-heading">
                  <h2>{new Date(item.createdAtUnixMs).toLocaleString()}</h2>
                  <span className={`status-badge ${item.state}`}>
                    {t(`recordings.state.${item.state}`)}
                  </span>
                </div>
                <p>
                  {item.width} × {item.height} · {(item.targetFpsNumerator / item.targetFpsDenominator).toFixed(0)} fps · {formatRecordingDuration(item.durationMs)} · {formatRecordingBytes(item.byteLength)}
                </p>
              </div>
              <button className="danger-link" type="button" disabled={busyKey !== null} onClick={() => setConfirmDelete(item.sessionId)}>
                {t("recordings.delete")}
              </button>
            </div>
            {item.state === "interrupted" && (
              <p className="recovery-note">{t("recordings.recoveryNote")}</p>
            )}
            {item.artifacts.length > 0 ? (
              <ul className="artifact-list">
                {item.artifacts.map((artifact) => (
                  <ArtifactRow
                    key={artifact.artifactId}
                    session={item}
                    artifact={artifact}
                    busyKey={busyKey}
                    onExport={(sessionId, artifactId) => void runArtifact(sessionId, artifactId, services.exportArtifact)}
                    onReveal={(sessionId, artifactId) => void runArtifact(sessionId, artifactId, services.revealArtifact)}
                  />
                ))}
              </ul>
            ) : (
              <p className="no-segments">{t("recordings.noSegments")}</p>
            )}
            {confirmDelete === item.sessionId && (
              <div className="delete-confirm" role="alertdialog" aria-label={t("recordings.deleteConfirm") }>
                <span>{t("recordings.deleteConfirm")}</span>
                <div>
                  <button type="button" onClick={() => setConfirmDelete(null)}>{t("recordings.cancel")}</button>
                  <button className="danger" type="button" disabled={busyKey !== null} onClick={() => void remove(item.sessionId)}>{t("recordings.delete")}</button>
                </div>
              </div>
            )}
          </article>
        ))}
      </section>
    </main>
  );
}
