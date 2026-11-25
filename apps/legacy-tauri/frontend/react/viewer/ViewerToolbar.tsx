import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Copy, Droplets, Expand, GripHorizontal, Hand, Languages, Minus, PenLine, Pin, Plus, Redo2, Save, ScanLine, TextSearch, Trash2, Undo2 } from "lucide-react";
import { clampToolbarPosition } from "../shared/toolbarPlacement";
import { COLORS, MAX_STROKE, MIN_STROKE, TOOL_GROUPS } from "../capture-overlay/tools";
import { t } from "../shared/i18n";
import type { Tool } from "../annotation/types";
import type { Size } from "./geometry";

export type Panel = "ocr" | "scan" | "translation" | "color";
type Props = {
  bounds: Size; blocked: boolean; canEdit: boolean; canScan: boolean; sensitive: boolean;
  pinUncertain: boolean;
  tool: Tool | "pan" | "color"; setTool: (tool: Tool | "pan" | "color") => void;
  panel: Panel | null; openPanel: (panel: Panel) => void;
  color: string; setColor: (color: string) => void; stroke: number; setStroke: (stroke: number) => void;
  text: string; setText: (text: string) => void; canUndo: boolean; canRedo: boolean; selected: boolean;
  undo: () => void; redo: () => void; remove: () => void;
  percentage: number; fit: () => void; actual: () => void; zoom: (factor: number) => void;
  copy: () => void; save: () => void; pin: () => void;
};

function Floating({ bounds, blocked, children }: { bounds: Size; blocked: boolean; children: ReactNode }) {
  const element = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 490, height: 83 });
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const active = useRef<{ id: number; x: number; y: number; left: number; top: number; target: HTMLElement } | null>(null);
  const limit = (point: { left: number; top: number }) => clampToolbarPosition(point, size, { x: 0, y: 0, ...bounds });
  const spot = limit(position || { left: (bounds.width - size.width) / 2, top: bounds.height - size.height - 12 });
  const latest = useRef({ limit, blocked }); latest.current = { limit, blocked };
  useLayoutEffect(() => {
    const measure = () => { const rect = element.current!.getBoundingClientRect(); if (rect.width && rect.height) setSize(old => old.width === rect.width && old.height === rect.height ? old : { width: rect.width, height: rect.height }); };
    measure(); const observer = new ResizeObserver(measure); observer.observe(element.current!); return () => observer.disconnect();
  }, []);
  function stop() { const item = active.current; active.current = null; if (item?.target.hasPointerCapture?.(item.id)) item.target.releasePointerCapture(item.id); }
  useEffect(() => { if (blocked) stop(); }, [blocked]);
  useEffect(() => {
    const move = (event: PointerEvent) => { const item = active.current; if (item && event.pointerId === item.id && !latest.current.blocked) setPosition(latest.current.limit({ left: item.left + event.clientX - item.x, top: item.top + event.clientY - item.y })); };
    const end = (event: PointerEvent) => { if (active.current?.id === event.pointerId) stop(); };
    const hidden = () => { if (document.hidden) stop(); };
    window.addEventListener("pointermove", move); window.addEventListener("pointerup", end); window.addEventListener("pointercancel", end); window.addEventListener("blur", stop); document.addEventListener("visibilitychange", hidden);
    return () => { stop(); window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", end); window.removeEventListener("pointercancel", end); window.removeEventListener("blur", stop); document.removeEventListener("visibilitychange", hidden); };
  }, []);
  return <div ref={element} className="viewer-floating-toolbar" style={{ left: spot.left, top: spot.top }} role="group" aria-label={t("viewer.tools")}>
    <button type="button" className="viewer-toolbar-grip" title={t("viewer.moveToolbar")} aria-label={t("viewer.moveToolbar")} disabled={blocked}
      onPointerDown={event => { if (blocked || event.button !== 0) return; event.preventDefault(); event.currentTarget.focus(); active.current = { id: event.pointerId, x: event.clientX, y: event.clientY, ...spot, target: event.currentTarget }; try { event.currentTarget.setPointerCapture(event.pointerId); } catch { /* window fallback */ } }}
      onLostPointerCapture={stop} onDoubleClick={() => { if (!blocked) setPosition(null); }}
      onKeyDown={event => { if (blocked) return; if (event.key === "Home") { event.preventDefault(); setPosition(null); } else if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) { event.preventDefault(); setPosition(limit({ left: spot.left + (event.key === "ArrowLeft" ? -16 : event.key === "ArrowRight" ? 16 : 0), top: spot.top + (event.key === "ArrowUp" ? -16 : event.key === "ArrowDown" ? 16 : 0) })); } }}><GripHorizontal size={14} /></button>
    {children}
  </div>;
}

export function ViewerToolbar(props: Props) {
  const [drawing, setDrawing] = useState(false);
  const button = (label: string, icon: ReactNode, action: () => void, disabled = false, active = false) => <button type="button" title={label} aria-label={label} aria-pressed={active} className={active ? "is-active" : ""} disabled={props.blocked || disabled} onClick={action}>{icon}</button>;
  return <Floating bounds={props.bounds} blocked={props.blocked}>
    <div className="viewer-toolbar-row viewer-primary-tools">
      {button(t("viewer.pan"), <Hand size={17} />, () => props.setTool("pan"), false, props.tool === "pan")}
      {button(t("viewer.draw"), <PenLine size={17} />, () => { setDrawing(!drawing); if (!drawing) props.setTool("pen"); else props.setTool("pan"); }, !props.canEdit, drawing)}
      <span className="viewer-control-separator" />
      {button(t("viewer.ocr"), <TextSearch size={17} />, () => props.openPanel("ocr"), false, props.panel === "ocr")}
      {button(t("viewer.scan"), <ScanLine size={17} />, () => props.openPanel("scan"), !props.canScan, props.panel === "scan")}
      {button(t("viewer.translation"), <Languages size={17} />, () => props.openPanel("translation"), false, props.panel === "translation")}
      {button(t("viewer.color"), <Droplets size={17} />, () => props.openPanel("color"), false, props.panel === "color")}
      <span className="viewer-control-separator" />
      {button(t("viewer.copyImage"), <Copy size={17} />, props.copy)}
      {button(t("viewer.save"), <Save size={17} />, props.save)}
      {button(t("viewer.pin"), <Pin size={17} />, props.pin, props.pinUncertain)}
    </div>
    {drawing && <>
      <div className="viewer-toolbar-row viewer-drawing-tools">{TOOL_GROUPS.map(group => <span className="viewer-tool-group" key={group.titleKey}>{group.tools.filter(item => item.id !== "select").map(item => <span key={item.id}>{button(t(item.labelKey), item.icon, () => props.setTool(item.id as Tool), !props.canEdit, props.tool === item.id)}</span>)}</span>)}</div>
      <div className="viewer-toolbar-row">
        <div className="viewer-swatches">{COLORS.map(color => <button type="button" key={color} disabled={props.blocked} style={{ backgroundColor: color }} aria-label={t("capture.color", { color })} aria-pressed={color === props.color} onClick={() => props.setColor(color)} />)}</div>
        <input type="range" aria-label={t("capture.size")} min={MIN_STROKE} max={MAX_STROKE} value={props.stroke} disabled={props.blocked} onChange={event => props.setStroke(Number(event.target.value))} />
        {button(t("capture.undo"), <Undo2 size={16} />, props.undo, !props.canUndo)}
        {button(t("capture.redo"), <Redo2 size={16} />, props.redo, !props.canRedo)}
        {button(t("capture.deleteObject"), <Trash2 size={16} />, props.remove, !props.selected)}
      </div>
      {props.tool === "text" && <label className="viewer-toolbar-text">{t("capture.text")}<input value={props.text} maxLength={16384} disabled={props.blocked} onChange={event => props.setText(event.target.value)} /></label>}
    </>}
    <div className="viewer-toolbar-row viewer-zoom-controls">
      {button(t("viewer.zoomOut"), <Minus size={16} />, () => props.zoom(.8))}<output aria-label={t("viewer.zoomLevel")}>{props.percentage}%</output>
      {button(t("viewer.zoomIn"), <Plus size={16} />, () => props.zoom(1.25))}<span className="viewer-control-separator" />
      {button(t("viewer.fit"), <Expand size={16} />, props.fit)}{button(t("viewer.actual"), <span>1:1</span>, props.actual)}
    </div>
  </Floating>;
}
