import { createRoot, type Root } from "react-dom/client";
import { ClipboardWorkspace } from "./ClipboardWorkspace";
import { TranslationPanel } from "./TranslationPanel";

let root: Root | null = null;

export function mountClipboardWorkspace(element: HTMLElement): void {
  root?.unmount();
  root = createRoot(element);
  root.render(<ClipboardWorkspace />);
}

let translationRoot: Root | null = null;

export function mountTranslationPanel(element: HTMLElement): void {
  translationRoot?.unmount();
  translationRoot = createRoot(element);
  translationRoot.render(<TranslationPanel />);
}
