import { createRoot } from "react-dom/client";
import { AippPage } from "../../src/components/AippPage";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { appStorageKey } from "../../src/lib/product-identity";
import "../../src/index.css";

const params = new URLSearchParams(location.search);
const lang = params.get("lang") === "en" ? "en" : "zh";
document.documentElement.lang = lang;
document.documentElement.dataset.theme = params.get("theme") || "light";
localStorage.setItem(appStorageKey("monitor.aipp.selectedSkill"), "example_collection");
const apiFetch = (url: string, init?: RequestInit) => fetch(url, init);
const t = (zh: string, en: string) => lang === "zh" ? zh : en;
createRoot(document.getElementById("root")!).render(
  <UiDialogProvider>
    <main className="theme-shell min-h-screen p-4 xl:pl-60">
      <AippPage lang={lang} t={t} apiFetch={apiFetch} onOpenAgent={() => {}} onOpenSkillStore={() => {}} />
    </main>
  </UiDialogProvider>,
);
