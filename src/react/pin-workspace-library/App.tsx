import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Bookmark, Image, Lock, Pin, Plus, Trash2, Type, X } from "lucide-react";
import {
  assignPinWorkspaceLibraryGroup,
  closePinWorkspaceLibrary,
  createPinWorkspaceLibraryGroup,
  deletePinWorkspaceLibraryGroup,
  getPinWorkspaceThumbnail,
  listPinWorkspaceLibrary,
  onPinWorkspaceLibraryChanged,
  pinWorkspaceLibraryReady,
  removePinWorkspaceLibraryItem,
  renamePinWorkspaceLibraryGroup,
  showPinWorkspaceItem,
  startPinWorkspaceLibraryDrag,
} from "../../js/api.ts";
import type {
  PinWorkspaceGroup,
  PinWorkspaceLibraryItem,
  PinWorkspaceLibrarySnapshot,
} from "../../js/ipc-types.ts";
import { t } from "../../i18n/i18n.js";

type Filter = "all" | "ungrouped" | number;

export type PinWorkspaceLibraryServices = {
  ready: typeof pinWorkspaceLibraryReady;
  list: typeof listPinWorkspaceLibrary;
  thumbnail: typeof getPinWorkspaceThumbnail;
  show: typeof showPinWorkspaceItem;
  assignGroup: typeof assignPinWorkspaceLibraryGroup;
  remove: typeof removePinWorkspaceLibraryItem;
  createGroup: typeof createPinWorkspaceLibraryGroup;
  renameGroup: typeof renamePinWorkspaceLibraryGroup;
  deleteGroup: typeof deletePinWorkspaceLibraryGroup;
  startDrag: typeof startPinWorkspaceLibraryDrag;
  close: typeof closePinWorkspaceLibrary;
  changed: typeof onPinWorkspaceLibraryChanged;
};

const defaultServices: PinWorkspaceLibraryServices = {
  ready: pinWorkspaceLibraryReady,
  list: listPinWorkspaceLibrary,
  thumbnail: getPinWorkspaceThumbnail,
  show: showPinWorkspaceItem,
  assignGroup: assignPinWorkspaceLibraryGroup,
  remove: removePinWorkspaceLibraryItem,
  createGroup: createPinWorkspaceLibraryGroup,
  renameGroup: renamePinWorkspaceLibraryGroup,
  deleteGroup: deletePinWorkspaceLibraryGroup,
  startDrag: startPinWorkspaceLibraryDrag,
  close: closePinWorkspaceLibrary,
  changed: onPinWorkspaceLibraryChanged,
};

function WorkspaceThumbnail({ item, load }: {
  item: PinWorkspaceLibraryItem;
  load: (id: number) => Promise<string | null>;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const element = host.current;
    if (!element || typeof IntersectionObserver === "undefined") {
      setNear(true);
      return;
    }
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) {
        setNear(true);
        observer.disconnect();
      }
    }, { rootMargin: "160px" });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!near || item.contentType !== "image") return;
    let cancelled = false;
    setFailed(false);
    void load(item.id).then((encoded) => {
      if (!cancelled && encoded) setSource(`data:image/png;base64,${encoded}`);
    }).catch(() => {
      if (!cancelled) setFailed(true);
    });
    return () => { cancelled = true; };
  }, [item.contentType, item.id, load, near]);

  if (item.contentType !== "image") {
    return (
      <div ref={host} className="workspace-text-preview">
        <Type size={24} aria-hidden="true" />
        <p>{item.previewText || t("pinLibrary.emptyText")}</p>
      </div>
    );
  }
  return (
    <div ref={host} className="workspace-image-preview">
      {source
        ? <img src={source} alt={t("pinLibrary.imageAlt")} />
        : <Image size={28} aria-label={failed ? t("pinLibrary.thumbnailFailed") : t("pinLibrary.thumbnailLoading")} />}
    </div>
  );
}

function itemMeta(item: PinWorkspaceLibraryItem): string {
  const dimensions = `${Math.round(item.contentWidth)} × ${Math.round(item.contentHeight)}`;
  const date = new Date(item.updatedAt * 1000).toLocaleString();
  return `${dimensions} · ${date}`;
}

export function App({ services = defaultServices }: { services?: PinWorkspaceLibraryServices } = {}) {
  const generation = useRef(0);
  const confirmButton = useRef<HTMLButtonElement>(null);
  const [snapshot, setSnapshot] = useState<PinWorkspaceLibrarySnapshot>({ groups: [], items: [] });
  const [filter, setFilter] = useState<Filter>("all");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(false);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState<number | null>(null);
  const [confirmGroupDelete, setConfirmGroupDelete] = useState(false);
  const [newGroup, setNewGroup] = useState("");
  const [renameGroup, setRenameGroup] = useState("");

  const load = useCallback(async () => {
    const current = ++generation.current;
    setLoading(true);
    setError(false);
    try {
      const next = await services.list();
      if (current !== generation.current) return;
      setSnapshot(next);
      setFilter((value) => typeof value === "number" && !next.groups.some((group) => group.id === value) ? "all" : value);
    } catch {
      if (current === generation.current) setError(true);
    } finally {
      if (current === generation.current) setLoading(false);
    }
  }, [services]);

  useEffect(() => {
    void load().finally(() => services.ready().catch(() => undefined));
    return () => { generation.current += 1; };
  }, [load, services]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    services.changed(() => { void load(); }).then((stop) => {
      if (cancelled) stop();
      else unlisten = stop;
    }).catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [load, services]);

  useEffect(() => {
    const selected = typeof filter === "number"
      ? snapshot.groups.find((group) => group.id === filter)
      : null;
    setRenameGroup(selected?.name || "");
    setConfirmGroupDelete(false);
  }, [filter, snapshot.groups]);

  useEffect(() => {
    if (confirmRemove != null || confirmGroupDelete) confirmButton.current?.focus();
  }, [confirmGroupDelete, confirmRemove]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (confirmRemove != null) setConfirmRemove(null);
      else if (confirmGroupDelete) setConfirmGroupDelete(false);
      else void services.close();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [confirmGroupDelete, confirmRemove, services]);

  const visibleItems = useMemo(() => snapshot.items.filter((item) => {
    if (filter === "all") return true;
    if (filter === "ungrouped") return item.groupId == null;
    return item.groupId === filter;
  }), [filter, snapshot.items]);

  const run = useCallback(async (key: string, action: () => Promise<void>) => {
    if (busyKey) return false;
    setBusyKey(key);
    setError(false);
    try {
      await action();
      return true;
    } catch {
      setError(true);
      return false;
    } finally {
      setBusyKey(null);
    }
  }, [busyKey]);

  const show = (id: number) => void run(`show:${id}`, async () => {
    await services.show(id);
    setSnapshot((value) => ({
      ...value,
      items: value.items.map((item) => item.id === id ? { ...item, open: true } : item),
    }));
  });

  const assign = (id: number, groupId: number | null) => void run(`assign:${id}`, async () => {
    await services.assignGroup(id, groupId);
    setSnapshot((value) => ({
      ...value,
      items: value.items.map((item) => item.id === id ? { ...item, groupId } : item),
    }));
  });

  const remove = (id: number) => void run(`remove:${id}`, async () => {
    await services.remove(id);
    setSnapshot((value) => ({ ...value, items: value.items.filter((item) => item.id !== id) }));
    setConfirmRemove(null);
  });

  const createGroup = () => void run("create-group", async () => {
    const group = await services.createGroup(newGroup);
    setSnapshot((value) => ({
      ...value,
      groups: value.groups.some((candidate) => candidate.id === group.id)
        ? value.groups.map((candidate) => candidate.id === group.id ? group : candidate)
        : [...value.groups, group],
    }));
    setNewGroup("");
    setFilter(group.id);
  });

  const selectedGroup = typeof filter === "number"
    ? snapshot.groups.find((group) => group.id === filter) ?? null
    : null;

  const renameSelectedGroup = () => {
    if (!selectedGroup) return;
    void run("rename-group", async () => {
      if (!await services.renameGroup(selectedGroup.id, renameGroup)) throw new Error("missing group");
      setSnapshot((value) => ({
        ...value,
        groups: value.groups.map((group) => group.id === selectedGroup.id ? { ...group, name: renameGroup.trim() } : group),
      }));
    });
  };

  const deleteSelectedGroup = () => {
    if (!selectedGroup) return;
    void run("delete-group", async () => {
      if (!await services.deleteGroup(selectedGroup.id)) throw new Error("missing group");
      setSnapshot((value) => ({
        groups: value.groups.filter((group) => group.id !== selectedGroup.id),
        items: value.items.map((item) => item.groupId === selectedGroup.id ? { ...item, groupId: null } : item),
      }));
      setFilter("ungrouped");
      setConfirmGroupDelete(false);
    });
  };

  const filterButton = (value: Filter, label: string, count: number) => (
    <button type="button" className={filter === value ? "active" : ""} onClick={() => setFilter(value)}>
      <span>{label}</span><small>{count}</small>
    </button>
  );

  return (
    <main className="pin-library-shell">
      <header className="pin-library-titlebar" onMouseDown={(event) => {
        if (event.button !== 0 || (event.target as Element).closest("button,input,select,textarea,a")) return;
        event.preventDefault();
        void services.startDrag();
      }}>
        <div>
          <h1>{t("pinLibrary.title")}</h1>
          <p>{t("pinLibrary.subtitle")}</p>
        </div>
        <button className="window-close" type="button" aria-label={t("pinLibrary.close")} onClick={() => void services.close()}><X size={20} /></button>
      </header>

      <div className="pin-library-body">
        <aside className="workspace-sidebar" aria-label={t("pinLibrary.groups")}>
          <nav>
            {filterButton("all", t("pinLibrary.all"), snapshot.items.length)}
            {filterButton("ungrouped", t("pinLibrary.ungrouped"), snapshot.items.filter((item) => item.groupId == null).length)}
            {snapshot.groups.map((group) => (
              <div key={group.id}>{filterButton(group.id, group.name, snapshot.items.filter((item) => item.groupId === group.id).length)}</div>
            ))}
          </nav>

          <form className="group-create" onSubmit={(event) => { event.preventDefault(); createGroup(); }}>
            <input value={newGroup} onChange={(event) => setNewGroup(event.target.value)} maxLength={64} placeholder={t("pinLibrary.newGroup")} />
            <button type="submit" disabled={!newGroup.trim() || busyKey !== null} aria-label={t("pinLibrary.createGroup")}><Plus size={16} /></button>
          </form>

          {selectedGroup && (
            <section className="group-manage">
              <label>{t("pinLibrary.renameGroup")}</label>
              <input value={renameGroup} onChange={(event) => setRenameGroup(event.target.value)} maxLength={64} />
              <div>
                <button type="button" disabled={!renameGroup.trim() || busyKey !== null} onClick={renameSelectedGroup}>{t("pinLibrary.rename")}</button>
                <button className="danger-link" type="button" disabled={busyKey !== null} onClick={() => setConfirmGroupDelete(true)}>{t("pinLibrary.delete")}</button>
              </div>
              {confirmGroupDelete && (
                <div className="inline-confirm" role="alertdialog" aria-label={t("pinLibrary.deleteGroupConfirm")}>
                  <span>{t("pinLibrary.deleteGroupConfirm")}</span>
                  <button type="button" onClick={() => setConfirmGroupDelete(false)}>{t("pinLibrary.cancel")}</button>
                  <button ref={confirmButton} className="danger" type="button" onClick={deleteSelectedGroup}>{t("pinLibrary.delete")}</button>
                </div>
              )}
            </section>
          )}
        </aside>

        <section className="workspace-content" aria-live="polite">
          {error && <div className="error-banner" role="status"><span>{t("pinLibrary.error")}</span><button type="button" onClick={() => void load()}>{t("pinLibrary.retry")}</button></div>}
          {loading && <div className="library-state">{t("pinLibrary.loading")}</div>}
          {!loading && !error && visibleItems.length === 0 && (
            <div className="library-state empty-state">
              <Bookmark size={34} aria-hidden="true" />
              <h2>{t("pinLibrary.emptyTitle")}</h2>
              <p>{t("pinLibrary.emptyBody")}</p>
            </div>
          )}
          {!loading && visibleItems.length > 0 && (
            <div className="workspace-grid">
              {visibleItems.map((item) => (
                <article className="workspace-card" key={item.id}>
                  <WorkspaceThumbnail item={item} load={services.thumbnail} />
                  <div className="workspace-card-copy">
                    <div className="workspace-card-heading">
                      <strong>{item.contentType === "image" ? t("pinLibrary.image") : t("pinLibrary.text")}</strong>
                      {item.open && <span className="open-badge"><Pin size={11} />{t("pinLibrary.open")}</span>}
                      {item.locked && <Lock size={13} aria-label={t("pinLibrary.locked")} />}
                    </div>
                    <span>{itemMeta(item)}</span>
                  </div>
                  <select aria-label={t("pinLibrary.groupForItem")} value={item.groupId ?? ""} disabled={busyKey !== null} onChange={(event) => assign(item.id, event.target.value ? Number(event.target.value) : null)}>
                    <option value="">{t("pinLibrary.ungrouped")}</option>
                    {snapshot.groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}
                  </select>
                  <div className="workspace-card-actions">
                    <button className="primary" type="button" disabled={busyKey !== null} onClick={() => show(item.id)}>{item.open ? t("pinLibrary.focus") : t("pinLibrary.show")}</button>
                    <button className="icon-danger" type="button" disabled={busyKey !== null} aria-label={t("pinLibrary.remove")} onClick={() => setConfirmRemove(item.id)}><Trash2 size={16} /></button>
                  </div>
                  {confirmRemove === item.id && (
                    <div className="inline-confirm card-confirm" role="alertdialog" aria-label={t("pinLibrary.removeConfirm")}>
                      <span>{item.open ? t("pinLibrary.removeOpenConfirm") : t("pinLibrary.removeConfirm")}</span>
                      <button type="button" onClick={() => setConfirmRemove(null)}>{t("pinLibrary.cancel")}</button>
                      <button ref={confirmButton} className="danger" type="button" onClick={() => remove(item.id)}>{t("pinLibrary.remove")}</button>
                    </div>
                  )}
                </article>
              ))}
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
