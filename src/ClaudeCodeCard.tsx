// Settings › Claude Code: Skylimit as Claude Code's status line, so Claude's
// limits come from the terminal (iTerm2, Terminal, …) the way they come from
// T3 Code. The shell edits only `statusLine` in Claude Code's settings.json
// (src-tauri/src/claude_code.rs).

import { useEffect, useState } from "react";

import { claudeCodeStatus, connectClaudeCode, disconnectClaudeCode } from "./api";
import { formatAgo } from "./format";
import type { ClaudeCodeStatus } from "./types";

/** The status refreshes this often while Settings are open (last reading age). */
const REFRESH_MS = 30_000;

export function ClaudeCodeCard() {
  const [status, setStatus] = useState<ClaudeCodeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const load = () =>
      claudeCodeStatus().then(
        (s) => alive && setStatus(s),
        (err: unknown) => alive && setError(String(err)),
      );
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);

  const act = async (action: () => Promise<ClaudeCodeStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await action());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card settings-card" aria-labelledby="claude-code-title">
      <h2 id="claude-code-title">Claude Code v terminálu</h2>
      <p className="hint">
        Limity Claude z Claude Code v iTermu2, Terminálu nebo jiném terminálu. Skylimit se stane jeho stavovým řádkem a po
        každé odpovědi si z něj vezme jen procenta limitů.
      </p>
      {status === null ? (
        !error && <p className="hint">Zjišťuji, jestli je Claude Code připojený…</p>
      ) : (
        <Body status={status} busy={busy} onConnect={() => void act(connectClaudeCode)} onDisconnect={() => void act(disconnectClaudeCode)} />
      )}
      {error && (
        <p className="hint err-text" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function Body({
  status,
  busy,
  onConnect,
  onDisconnect,
}: {
  status: ClaudeCodeStatus;
  busy: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
}) {
  if (status.state === "missing") {
    return <p className="hint">Claude Code tu není. Až ho nainstaluješ a jednou spustíš, půjde ho tady připojit.</p>;
  }
  const connected = status.state === "on";
  let line: string;
  if (status.state === "moved") line = "Skylimit je teď jinde než při připojení. Připoj ho znovu.";
  else if (!connected) line = "Nepřipojeno.";
  else if (status.lastReadingAt) line = `Připojeno, poslední limity ${formatAgo(Date.now() / 1000 - status.lastReadingAt)}.`;
  else line = "Připojeno. Limity přijdou s další odpovědí Claude Code.";

  return (
    <>
      <p className="claude-code-state" role="status">
        <i className={`dot ${connected ? "dot-on" : "dot-off"}`} aria-hidden="true" />
        {line}
      </p>
      <p className="hint">
        {status.ownStatusLine
          ? "Tvůj vlastní stavový řádek poběží dál, Skylimit ho jen spustí za sebe."
          : `Claude Code ${connected ? "ukazuje" : "pak ukáže"} dole v terminálu řádek s limity, třeba ☀ Claude 5h 24 % · týden 41 %.`}{" "}
        Mění se jen klíč statusLine v <span className="key">{status.settingsPath}</span>, Odpojit vrátí původní stav.
      </p>
      {status.problem && <p className="hint err-text">{status.problem}</p>}
      <div className="confirm">
        {connected ? (
          <button type="button" className="btn" disabled={busy} onClick={onDisconnect}>
            Odpojit
          </button>
        ) : (
          <>
            <button type="button" className="btn primary" disabled={busy || status.problem !== ""} onClick={onConnect}>
              {status.state === "moved" ? "Připojit znovu" : "Připojit"}
            </button>
            {status.state === "moved" && (
              <button type="button" className="btn" disabled={busy} onClick={onDisconnect}>
                Odpojit
              </button>
            )}
          </>
        )}
      </div>
    </>
  );
}
