import { createRoot } from "react-dom/client";
import "../../styles/themes.css";
import "../../styles/base.css";
import "../../styles/capture.css";
import { App } from "./App";

const root = document.getElementById("capture-root");

if (root) {
  createRoot(root).render(<App />);
}

