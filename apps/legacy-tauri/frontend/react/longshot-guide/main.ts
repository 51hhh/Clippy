import { parseGuideRect } from "./geometry";
import "./style.css";

const guide = document.getElementById("guide");
const rect = parseGuideRect(location.search, window.innerWidth, window.innerHeight);

if (guide && rect) {
  guide.style.left = `${rect.x}px`;
  guide.style.top = `${rect.y}px`;
  guide.style.width = `${rect.width}px`;
  guide.style.height = `${rect.height}px`;
  guide.dataset.ready = "true";
} else {
  guide?.remove();
}
