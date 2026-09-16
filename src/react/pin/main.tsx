import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { initializeReactI18n } from "../shared/i18n";
import "./pin.css";

void initializeReactI18n().then(() => {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
});
