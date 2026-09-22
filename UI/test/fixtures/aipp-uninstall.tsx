import { createRoot } from "react-dom/client";
import { AippPage } from "../../src/components/AippPage";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "en" ? "en" : "zh";
document.documentElement.lang = lang;
document.documentElement.dataset.theme = params.get("theme") || "light";
createRoot(document.getElementById("root")!).render(
  <UiDialogProvider><main className="theme-shell min-h-screen p-4">
    <AippPage lang={lang} t={(zh, en) => lang === "zh" ? zh : en}
      apiFetch={(url, init) => fetch(url, init)} onOpenAgent={() => {}}
      onOpenSkillStore={() => { document.documentElement.dataset.storeOpened = "true"; }}
      onSkillsChanged={() => { document.documentElement.dataset.skillsChanged = "true"; }} />
  </main></UiDialogProvider>,
);
