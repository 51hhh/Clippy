import { createRoot } from "react-dom/client";
import "../../styles/themes.css";
import "../../styles/base.css";
import "../../styles/capture.css";
import { App } from "./App";
import { initializeReactI18n } from "../shared/i18n";

const root = document.getElementById("capture-root");

if (root) {
  void initializeReactI18n().then(() => createRoot(root).render(<App />));
}
