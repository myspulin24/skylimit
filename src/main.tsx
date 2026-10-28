import { StrictMode, Suspense, lazy } from "react";
import { createRoot } from "react-dom/client";
import { isTauri } from "./api";
import { AppWindow } from "./AppWindow";
import { installAnimationPause } from "./hooks";
import { Widget } from "./Widget";
import "./styles/base.css";
import "./styles/character.css";

// Dev helpers are split out so the fixtures never load inside Tauri.
const DevSwitcher = lazy(() => import("./dev/DevSwitcher"));
const MoodGallery = lazy(() => import("./dev/MoodGallery"));

type WindowKind = "widget" | "app" | "gallery";

const mockMode = !isTauri();
const requested = new URLSearchParams(location.search).get("window") ?? "widget";
const kind: WindowKind = requested === "app" ? "app" : requested === "gallery" && mockMode ? "gallery" : "widget";

const html = document.documentElement;
html.dataset.window = kind;
if (mockMode) html.classList.add("mock");
installAnimationPause();

const root = document.getElementById("root");
if (!root) throw new Error("#root missing in index.html");

createRoot(root).render(
  <StrictMode>
    {kind === "app" ? (
      <AppWindow />
    ) : kind === "gallery" ? (
      <Suspense fallback={null}>
        <MoodGallery />
      </Suspense>
    ) : (
      <Widget />
    )}
    {mockMode && (
      <Suspense fallback={null}>
        <DevSwitcher />
      </Suspense>
    )}
  </StrictMode>,
);
