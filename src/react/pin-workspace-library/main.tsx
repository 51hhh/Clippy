import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getPinWorkspaceLibrarySettings } from "../../js/api.ts";
import { init } from "../../i18n/i18n.js";
import { App } from "./App.tsx";
import "./pin-workspace-library.css";

void getPinWorkspaceLibrarySettings()
  .catch(() => null)
  .then((settings) => {
    init(settings?.language || "auto");
    document.documentElement.dataset.theme = settings?.theme || "light";
    createRoot(document.getElementById("root")!).render(
      <StrictMode>
        <App />
      </StrictMode>,
    );
  });
