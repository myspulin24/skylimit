import { useEffect, useState } from "react";
import { getSnapshot, onSnapshot } from "./api";
import type { Snapshot } from "./types";

export interface SnapshotState {
  snapshot: Snapshot | null;
  /** Date.now() when the snapshot arrived; makes "updated ago" tick locally. */
  receivedAt: number;
  error: string | null;
}

/** Latest snapshot: initial `get_snapshot` plus every `snapshot` event. */
export function useSnapshot(): SnapshotState {
  const [state, setState] = useState<SnapshotState>({ snapshot: null, receivedAt: Date.now(), error: null });

  useEffect(() => {
    let alive = true;
    const accept = (snapshot: Snapshot) => {
      if (!alive) return;
      setState((prev) =>
        // The initial fetch can resolve after a newer event; keep the newer one.
        prev.snapshot && prev.snapshot.generatedAt > snapshot.generatedAt
          ? prev
          : { snapshot, receivedAt: Date.now(), error: null },
      );
    };
    const unsubscribe = onSnapshot(accept);
    getSnapshot().then(accept, (err: unknown) => {
      if (alive) setState((prev) => ({ ...prev, error: String(err) }));
    });
    return () => {
      alive = false;
      unsubscribe();
    };
  }, []);

  return state;
}

/** Wall clock that re-renders every `intervalMs`; skips ticks while the page is hidden. */
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const tick = () => {
      if (!document.hidden) setNow(Date.now());
    };
    const id = window.setInterval(tick, intervalMs);
    document.addEventListener("visibilitychange", tick);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", tick);
    };
  }, [intervalMs]);
  return now;
}

/**
 * Pauses every CSS animation while the window is not visible (minimised,
 * hidden widget, covered tab). The class lives on <html>, see base.css.
 */
export function installAnimationPause(doc: Document = document): () => void {
  const sync = () => doc.documentElement.classList.toggle("anim-paused", doc.hidden);
  sync();
  doc.addEventListener("visibilitychange", sync);
  return () => doc.removeEventListener("visibilitychange", sync);
}
