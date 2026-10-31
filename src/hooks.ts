import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
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
/** Width of an element in CSS pixels, kept current while the window
 *  resizes. Charts draw their viewBox at this width, so a wide window gets
 *  wider bars at the same text size instead of a taller, scaled-up picture.
 *  `fallback` stands in until the first measurement (and in tests). */
export function useWidth<T extends Element>(fallback: number): [RefObject<T | null>, number] {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(fallback);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const w = Math.round(el.getBoundingClientRect().width);
      if (w > 0) setWidth(w);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

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
