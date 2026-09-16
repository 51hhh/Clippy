import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { clampToolbarPosition, fullWindowBounds } from "../shared/toolbarPlacement";

/** jsdom 与首帧量不到尺寸时的兜底。 */
const FALLBACK_SIZE = { width: 176, height: 200 };

export type PinMenuItem = {
  id: string;
  label: string;
  /** 开关类项目的当前状态，画一个勾。`undefined` 表示这不是开关。 */
  checked?: boolean;
  danger?: boolean;
  onSelect: () => void;
};

type Props = {
  /** 右键落点，窗口坐标。 */
  at: { x: number; y: number };
  items: PinMenuItem[];
  onDismiss: () => void;
};

/**
 * 贴图窗口的右键菜单。
 *
 * WebKit 自带的那份网页菜单（重新加载、检查元素）已经在 GTK 信号层关掉了
 * （见 `src-tauri/src/webview_hardening.rs`），右键这个手势因此空出来给快速操作用。
 * 关掉 WebKit 菜单**不影响** DOM 的 `contextmenu` 事件，所以这里照常收得到。
 *
 * 菜单落点用 `clampToolbarPosition` 钳进视口：贴图窗口很小，在靠边的地方右键，
 * 菜单会有一半在窗口外——而这是个无边框窗口，超出去的部分直接被裁掉，不像浏览器
 * 那样能溢出到屏幕上。
 */
export function PinContextMenu(props: Props) {
  const panel = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(FALLBACK_SIZE);

  useLayoutEffect(() => {
    const element = panel.current;
    if (!element) return;
    const rect = element.getBoundingClientRect();
    const next = {
      width: rect.width || FALLBACK_SIZE.width,
      height: rect.height || FALLBACK_SIZE.height,
    };
    setSize((current) =>
      current.width === next.width && current.height === next.height ? current : next,
    );
  });

  // 点别处、按 Esc、滚轮缩放都该收起菜单。挂在 window 上并用捕获阶段：
  // 菜单项自己的 click 要先跑完，所以那边会 stopPropagation。
  useEffect(() => {
    function onPointerDown(event: PointerEvent) {
      if (panel.current?.contains(event.target as Node)) return;
      props.onDismiss();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        // 别让这次 Esc 继续传成"关闭贴图"。
        event.stopPropagation();
        props.onDismiss();
      }
    }
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("wheel", props.onDismiss, { passive: true });
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("wheel", props.onDismiss);
    };
  }, [props.onDismiss]);

  const spot = clampToolbarPosition(
    { left: props.at.x, top: props.at.y },
    size,
    // 菜单是右键当场弹的，用整个窗口做边界就够——它不像工具条那样长期停在某处，
    // 不值得为它多问一次后端（那是一趟 D-Bus）。
    fullWindowBounds({ width: window.innerWidth, height: window.innerHeight }),
    4,
  );

  return (
    <div
      ref={panel}
      className="pin-context-menu"
      role="menu"
      data-pin-controls
      style={{ left: spot.left, top: spot.top }}
    >
      {props.items.map((item) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          className={item.danger ? "danger" : undefined}
          aria-checked={item.checked}
          onClick={() => {
            item.onSelect();
            props.onDismiss();
          }}
        >
          <span className="pin-context-check" aria-hidden="true">
            {item.checked ? "✓" : ""}
          </span>
          {item.label}
        </button>
      ))}
    </div>
  );
}
