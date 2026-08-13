import { createRoot, type Root } from "react-dom/client";
import { ClipboardWorkspace } from "./ClipboardWorkspace";

let root: Root | null = null;

export function mountClipboardWorkspace(element: HTMLElement): void {
  root?.unmount();
  root = createRoot(element);
  root.render(<ClipboardWorkspace />);
}
