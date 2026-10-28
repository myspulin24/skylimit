// The only place that talks to the Tauri shell. Inside Tauri every call goes to
// a command from docs/snapshot-contract.md; in a plain browser (`npm run dev`)
// the same functions are served by the in-memory mock in ./mock.ts, which is
// loaded lazily so the fixtures never ship in the Tauri bundle's main chunk.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { FeedbackReceipt, FeedbackRequest, FeedbackStatus, Settings, Snapshot, UpdateStatus } from "./types";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && window.__TAURI_INTERNALS__ !== undefined;
}

type MockModule = typeof import("./mock");
let mockModule: Promise<MockModule> | null = null;
const mock = () => (mockModule ??= import("./mock"));

export async function getSnapshot(): Promise<Snapshot> {
  if (isTauri()) return invoke<Snapshot>("get_snapshot");
  return (await mock()).getSnapshot();
}

export async function getSettings(): Promise<Settings> {
  if (isTauri()) return invoke<Settings>("get_settings");
  return (await mock()).getSettings();
}

export async function setSettings(settings: Settings): Promise<void> {
  if (isTauri()) return invoke<void>("set_settings", { settings });
  return (await mock()).setSettings(settings);
}

export async function setClickThrough(on: boolean): Promise<void> {
  if (isTauri()) return invoke<void>("set_click_through", { on });
  return (await mock()).setClickThrough(on);
}

/** Widget background transparency in percent; saved and applied at once. */
export async function setWidgetTransparency(value: number): Promise<void> {
  if (isTauri()) return invoke<void>("set_widget_transparency", { value });
  return (await mock()).setWidgetTransparency(value);
}

/** Read every source now; fresh numbers arrive through the "snapshot" event. */
export async function refreshUsage(): Promise<void> {
  if (isTauri()) return invoke<void>("refresh_usage");
  return (await mock()).refreshUsage();
}

/** The app window with the feedback dialog open (widget button). */
export async function openFeedback(): Promise<void> {
  if (isTauri()) return invoke<void>("open_feedback");
  return (await mock()).openFeedback();
}

export async function openApp(): Promise<void> {
  if (isTauri()) return invoke<void>("open_app");
  return (await mock()).openApp();
}

export async function clearHistory(): Promise<void> {
  if (isTauri()) return invoke<void>("clear_history");
  return (await mock()).clearHistory();
}

export async function feedbackStatus(): Promise<FeedbackStatus> {
  if (isTauri()) return invoke<FeedbackStatus>("feedback_status");
  return (await mock()).feedbackStatus();
}

/** Resolves with the Worker's receipt; rejects with a sentence for the user. */
export async function sendFeedback(request: FeedbackRequest): Promise<FeedbackReceipt> {
  if (isTauri()) return invoke<FeedbackReceipt>("feedback_send", { request });
  return (await mock()).sendFeedback(request);
}

/** The tray's "Poslat zpětnou vazbu…" while the app window is already open. */
export function onOpenFeedback(cb: () => void): () => void {
  if (!isTauri()) return () => {};
  let disposed = false;
  let unlisten: (() => void) | null = null;
  listen("open-feedback", () => cb())
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((err: unknown) => console.error("open-feedback listener failed", err));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

export async function updateStatus(): Promise<UpdateStatus> {
  if (isTauri()) return invoke<UpdateStatus>("update_status");
  return (await mock()).updateStatus();
}

/** Manual "Zkontrolovat aktualizace": errors come back in the status. */
export async function checkForUpdate(): Promise<UpdateStatus> {
  if (isTauri()) return invoke<UpdateStatus>("update_check");
  return (await mock()).checkForUpdate();
}

/** Download, verify, install and restart. Resolves only if it failed. */
export async function installUpdate(): Promise<void> {
  if (isTauri()) return invoke<void>("update_install");
  return (await mock()).installUpdate();
}

/** Every status change (check, progress) as the "update" event. */
export function onUpdate(cb: (status: UpdateStatus) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  const ready: Promise<() => void> = isTauri()
    ? listen<UpdateStatus>("update", (event) => cb(event.payload))
    : mock().then((m) => m.subscribeUpdate(cb));
  ready
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((err: unknown) => console.error("update listener failed", err));
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** Subscribes to the "snapshot" event; returns a synchronous unsubscribe. */
export function onSnapshot(cb: (snapshot: Snapshot) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  const ready: Promise<() => void> = isTauri()
    ? listen<Snapshot>("snapshot", (event) => cb(event.payload))
    : mock().then((m) => m.subscribe(cb));
  ready
    .then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    })
    .catch((err: unknown) => console.error("snapshot listener failed", err));
  return () => {
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}
