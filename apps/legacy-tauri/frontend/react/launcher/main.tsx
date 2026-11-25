import { createRoot } from "react-dom/client";
import { init } from "../../i18n/i18n.js";
import type { ActionLauncherSettings } from "../../js/api.ts";
import { LauncherApp } from "./App";
import { launcherApi } from "./api";

const fallback: ActionLauncherSettings = {
  theme: "light",
  language: "auto",
  translationSourceLanguage: "auto",
  translationTargetLanguage: "en",
};

void launcherApi.settings().catch(() => fallback).then(settings => {
  init(settings.language);
  document.documentElement.dataset.theme = settings.theme;
  createRoot(document.getElementById("root")!).render(
    <LauncherApp settings={settings} />,
  );
});
