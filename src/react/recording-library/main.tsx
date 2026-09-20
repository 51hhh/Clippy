import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getRecordingLibrarySettings } from "../../js/api.ts";
import { init } from "../../i18n/i18n.js";
import { App } from "./App.tsx";
import "./recording-library.css";

void getRecordingLibrarySettings()
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
