// Settings › Widget: start Skylimit when the user logs in. Applies at once,
// like click-through: the system's own startup list is the only record, so
// there is nothing for "Uložit" to save (src-tauri/src/autostart.rs).

import { useEffect, useState } from "react";

import { autostartStatus, setAutostart } from "./api";
import { ToggleRow } from "./controls";
import type { AutostartStatus } from "./types";

export function AutostartCheck() {
  const [status, setStatus] = useState<AutostartStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    autostartStatus().then(
      (s) => alive && setStatus(s),
      (err: unknown) => alive && setError(String(err)),
    );
    return () => {
      alive = false;
    };
  }, []);

  const onChange = async (on: boolean) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await setAutostart(on));
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const on = status?.state === "on";
  let hint = "Widget se objeví sám, jakmile se přihlásíš do počítače. Projeví se hned.";
  if (status?.problem && !on) hint = status.problem;
  else if (status?.state === "moved") hint = "Po přihlášení se teď spouští Skylimit z jiné složky. Zaškrtnutím přepneš na tenhle.";

  return (
    <>
      <ToggleRow
        title="Spouštět po přihlášení do počítače"
        desc={hint}
        descClass={status?.problem && !on ? "err-text" : undefined}
        checked={on}
        disabled={status === null || busy || (status.problem !== "" && !on)}
        onChange={(next) => void onChange(next)}
      />
      {error && (
        <p className="row-note err-text" role="alert">
          {error}
        </p>
      )}
    </>
  );
}
