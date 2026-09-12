import { copy } from "./i18n";
import "./theme.css";

export function initialTheme(): "light" | "dark" {
  return localStorage.getItem("agent-runtime.monitor.themeMode") === "light" ? "light" : "dark";
}

export function ThemeToggle({ theme, onToggle, className = "" }: {
  theme: string; onToggle: () => void; className?: string;
}) {
  const label = theme === "dark" ? copy("切换到浅色外观") : copy("切换到深色外观");
  return <button type="button" className={className} data-desktop-theme-toggle
    aria-label={label} title={label} onClick={onToggle}>
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {theme === "dark" ? <><circle cx="12" cy="12" r="4" />
        <path d="M12 2v2m0 16v2M2 12h2m16 0h2M4.93 4.93l1.42 1.42m11.3 11.3 1.42 1.42M4.93 19.07l1.42-1.42m11.3-11.3 1.42-1.42" /></>
        : <path d="M20.9 13.1A9 9 0 0 1 10.9 3.1 9 9 0 1 0 20.9 13.1Z" />}
    </svg>
  </button>;
}
