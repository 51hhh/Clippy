import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { getViewerSettings } from "../../js/api";
import { init } from "../../i18n/i18n.js";

void getViewerSettings().catch(() => null).then(config => {
  init(config?.language || "auto");
  document.documentElement.dataset.theme = config?.theme || "light";
  createRoot(document.getElementById("root")!).render(<StrictMode><App /></StrictMode>);
});
