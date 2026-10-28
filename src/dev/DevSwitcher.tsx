// Browser-only state switcher (`npm run dev`): pick a fixture and a window.
// Loaded lazily and never rendered inside Tauri. `?clean` hides the bar but
// keeps the fake desktop from dev.css (screenshots).

import { FIXTURE_LABEL, FIXTURE_NAMES } from "../fixtures";
import "../styles/dev.css";

const WINDOWS = [
  { id: "widget", label: "widget" },
  { id: "app", label: "aplikace" },
  { id: "gallery", label: "nálady" },
] as const;

function href(windowId: string, mock: string | null): string {
  const params = new URLSearchParams();
  params.set("window", windowId);
  if (mock) params.set("mock", mock);
  return `?${params.toString()}`;
}

export default function DevSwitcher() {
  const params = new URLSearchParams(location.search);
  if (params.has("clean")) return null;
  const currentWindow = params.get("window") ?? "widget";
  const currentMock = params.get("mock") ?? "normal";
  return (
    <nav className="dev-switcher" aria-label="Ukázkové stavy (jen v prohlížeči)">
      <span className="dev-title">mock</span>
      <div className="dev-group">
        {WINDOWS.map((w) => (
          <a key={w.id} href={href(w.id, currentMock)} className={w.id === currentWindow ? "is-on" : undefined}>
            {w.label}
          </a>
        ))}
      </div>
      <div className="dev-group">
        {FIXTURE_NAMES.map((name) => (
          <a key={name} href={href(currentWindow, name)} className={name === currentMock ? "is-on" : undefined}>
            {FIXTURE_LABEL[name]}
          </a>
        ))}
      </div>
    </nav>
  );
}
