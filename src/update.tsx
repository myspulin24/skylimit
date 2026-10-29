// Updates in the UI: one status from the shell (src-tauri/src/updater.rs),
// shown as a banner in the app window, a dot on the widget's gear and a line
// in "O aplikaci". Installing is always a click; nothing restarts on its own.

import { Fragment, useEffect, useState, type ReactNode } from "react";
import { checkForUpdate, installUpdate, onUpdate, updateStatus } from "./api";
import type { UpdateStatus } from "./types";

/** Latest update status: initial fetch plus every "update" event. */
export function useUpdate(): UpdateStatus | null {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const unsubscribe = onUpdate((s) => alive && setStatus(s));
    updateStatus().then(
      (s) => alive && setStatus((prev) => prev ?? s),
      () => {},
    );
    return () => {
      alive = false;
      unsubscribe();
    };
  }, []);
  return status;
}

/** An offer the banner and the widget dot should show. */
export function hasOffer(status: UpdateStatus | null): boolean {
  if (!status?.version) return false;
  return status.phase === "available" || status.phase === "downloading" || status.phase === "installing" || status.phase === "error";
}

/** **bold** inside a line of release notes. */
function inline(text: string): ReactNode[] {
  return text.split(/(\*\*[^*]+\*\*)/g).map((part, i) =>
    part.startsWith("**") && part.endsWith("**") ? <b key={i}>{part.slice(2, -2)}</b> : <Fragment key={i}>{part}</Fragment>,
  );
}

/** The Markdown CHANGELOG.md is written in: `###` headings and `- ` bullets
 *  (with indented continuation lines), bold. Anything else is plain text. */
export function ReleaseNotes({ notes }: { notes: string }) {
  const blocks: ReactNode[] = [];
  let items: string[] = [];
  const flush = () => {
    if (items.length) {
      blocks.push(
        <ul key={`l${blocks.length}`}>
          {items.map((item, i) => (
            <li key={i}>{inline(item)}</li>
          ))}
        </ul>,
      );
      items = [];
    }
  };
  for (const raw of notes.split(/\r?\n/)) {
    const line = raw.trimEnd();
    if (line.startsWith("### ")) {
      flush();
      blocks.push(<h4 key={`h${blocks.length}`}>{line.slice(4)}</h4>);
    } else if (line.startsWith("- ")) {
      flush();
      items.push(line.slice(2));
    } else if (/^\s+\S/.test(line) && items.length) {
      items[items.length - 1] += ` ${line.trim()}`;
    } else if (line.trim() === "---") {
      flush();
      break; // what follows is the download hint for GitHub, not for the app
    } else if (line.trim()) {
      flush();
      blocks.push(<p key={`p${blocks.length}`}>{inline(line.trim())}</p>);
    }
  }
  flush();
  return <div className="release-notes">{blocks}</div>;
}

/** Banner in the app window while a newer version is on offer. */
export function UpdateBanner({ status }: { status: UpdateStatus | null }) {
  const [open, setOpen] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  if (!status || !hasOffer(status)) return null;

  const install = () => {
    setFailed(null);
    installUpdate().catch((err: unknown) => setFailed(String(err)));
  };
  const busy = status.phase === "downloading" || status.phase === "installing";
  const percent = Math.round((status.progress ?? 0) * 100);

  return (
    <section className="update-banner" role="status" aria-label="Aktualizace">
      <div className="update-row">
        <span className="update-badge" aria-hidden="true">
          ↑
        </span>
        <p className="update-text">
          {status.phase === "downloading" ? (
            <>
              Stahuji <b>Skylimit {status.version}</b>… {percent}&nbsp;%
            </>
          ) : status.phase === "installing" ? (
            <>Instaluji. Skylimit se za chvíli spustí znovu.</>
          ) : (
            <>
              Je k dispozici <b>Skylimit {status.version}</b>. Teď máš {status.current}.
            </>
          )}
        </p>
        <div className="update-actions">
          {status.notes && (
            <button type="button" className="btn small" aria-expanded={open} onClick={() => setOpen(!open)}>
              {open ? "Skrýt novinky" : "Co je nového"}
            </button>
          )}
          <button type="button" className="btn small primary" onClick={install} disabled={busy}>
            Aktualizovat a restartovat
          </button>
        </div>
      </div>
      {busy && status.phase === "downloading" && (
        <div className="update-progress" aria-hidden="true">
          <span className="update-progress-fill" style={{ width: `${percent}%` }} />
        </div>
      )}
      {(failed ?? (status.phase === "error" ? status.error : null)) && (
        <p className="status err">{failed ?? status.error}</p>
      )}
      {open && status.notes && <ReleaseNotes notes={status.notes} />}
    </section>
  );
}

function ago(epoch: number): string {
  const minutes = Math.max(0, Math.round((Date.now() / 1000 - epoch) / 60));
  if (minutes < 1) return "právě teď";
  if (minutes < 60) return `před ${minutes} min`;
  const hours = Math.round(minutes / 60);
  return hours < 24 ? `před ${hours} h` : `před ${Math.round(hours / 24)} d`;
}

/** Settings › O aplikaci: state of updates and a manual check, as one row. */
export function UpdateLine({ status }: { status: UpdateStatus | null }) {
  const [checking, setChecking] = useState(false);
  const check = () => {
    setChecking(true);
    checkForUpdate().finally(() => setChecking(false));
  };
  let text: string;
  if (!status) text = "Zjišťuji stav aktualizací…";
  else if (checking || status.phase === "checking") text = "Hledám novou verzi…";
  else if (hasOffer(status)) text = `Je k dispozici verze ${status.version}, aktualizovat jde nahoře v okně.`;
  else if (status.phase === "error") text = status.error ?? "Aktualizaci se nepodařilo zkontrolovat.";
  else if (status.phase === "upToDate") text = `Máš nejnovější verzi${status.checkedAt ? ` (kontrola ${ago(status.checkedAt)})` : ""}.`;
  else text = status.auto ? "Nová verze se hledá automaticky po spuštění a každých 6 hodin." : "Automatická kontrola je vypnutá.";

  return (
    <div className="row">
      <div className="row-text">
        <div className="row-title">Aktualizace</div>
        <p className={status?.phase === "error" && !hasOffer(status) ? "row-desc err-text" : "row-desc"} role="status">
          {text}
        </p>
      </div>
      <div className="row-control">
        <button
          type="button"
          className="btn"
          aria-label="Zkontrolovat aktualizace"
          onClick={check}
          disabled={checking || status?.phase === "checking"}
        >
          Zkontrolovat
        </button>
      </div>
    </div>
  );
}
