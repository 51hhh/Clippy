import { useEffect, useRef, useState } from "react";
import type { PinWorkspaceGroup } from "./types";
import { t } from "../shared/i18n";

type Props = {
  groups: PinWorkspaceGroup[];
  groupId: number | null;
  busy: boolean;
  error: string | null;
  onAssign: (groupId: number | null) => void;
  onCreate: (name: string) => Promise<boolean>;
  onRename: (id: number, name: string) => void;
  onDelete: (id: number) => void;
  onRemove: () => void;
  onDismiss: () => void;
};

/**
 * 已保存 Pin 的轻量工作区管理器。
 *
 * 它固定在当前无边框窗口内，并按 viewport 收缩。小图片的 Pin 可能只有百余像素宽，
 * 因而不能像普通浮层那样假设左右一定有 240px 空间。
 */
export function PinWorkspacePopover(props: Props) {
  const panel = useRef<HTMLElement>(null);
  const selected = props.groups.find((group) => group.id === props.groupId) ?? null;
  const [renameName, setRenameName] = useState(selected?.name ?? "");
  const [newName, setNewName] = useState("");

  useEffect(() => {
    setRenameName(selected?.name ?? "");
  }, [selected?.id, selected?.name]);

  useEffect(() => {
    function onPointerDown(event: PointerEvent) {
      if (!panel.current?.contains(event.target as Node)) props.onDismiss();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      props.onDismiss();
    }
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [props.onDismiss]);

  const trimmedRename = renameName.trim();
  const trimmedNew = newName.trim();
  async function createGroup() {
    if (await props.onCreate(trimmedNew)) setNewName("");
  }
  return (
    <section ref={panel} className="pin-workspace-popover" data-pin-controls aria-label={t("pin.workspaceManage")}>
      <header>
        <strong>{t("pin.workspaceManage")}</strong>
        <button type="button" aria-label={t("pin.workspaceCloseManager")} onClick={props.onDismiss}>×</button>
      </header>
      <label>
        <span>{t("pin.workspaceGroup")}</span>
        <select
          aria-label={t("pin.workspaceGroup")}
          value={props.groupId ?? ""}
          disabled={props.busy}
          onChange={(event) => props.onAssign(event.target.value ? Number(event.target.value) : null)}
        >
          <option value="">{t("pin.workspaceUngrouped")}</option>
          {props.groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}
        </select>
      </label>
      {selected && <>
        <label>
          <span>{t("pin.workspaceRenameGroup")}</span>
          <input
            aria-label={t("pin.workspaceRenameName")}
            value={renameName}
            maxLength={80}
            disabled={props.busy}
            onChange={(event) => setRenameName(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || !trimmedRename || trimmedRename === selected.name) return;
              event.preventDefault();
              props.onRename(selected.id, trimmedRename);
            }}
          />
        </label>
        <div className="pin-workspace-actions">
          <button type="button" disabled={props.busy || !trimmedRename || trimmedRename === selected.name}
            onClick={() => props.onRename(selected.id, trimmedRename)}>
            {t("pin.workspaceRenameGroup")}
          </button>
          <button type="button" className="danger" disabled={props.busy}
            onClick={() => props.onDelete(selected.id)}>
            {t("pin.workspaceDeleteGroup")}
          </button>
        </div>
      </>}
      <label>
        <span>{t("pin.workspaceNewGroup")}</span>
        <input
          aria-label={t("pin.workspaceNewGroupName")}
          value={newName}
          maxLength={80}
          disabled={props.busy}
          onChange={(event) => setNewName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== "Enter" || !trimmedNew) return;
            event.preventDefault();
            void createGroup();
          }}
        />
      </label>
      <button type="button" disabled={props.busy || !trimmedNew} onClick={() => void createGroup()}>
        {t("pin.workspaceCreateGroup")}
      </button>
      {props.error && <p className="pin-workspace-error" role="status">{props.error}</p>}
      <button type="button" className="pin-workspace-remove danger" disabled={props.busy} onClick={props.onRemove}>
        {t("pin.workspaceRemove")}
      </button>
    </section>
  );
}
