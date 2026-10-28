// "Zpětná vazba": a modal in the app window. The message goes through the
// shell (feedback_send) to the Worker, which files an issue in a private repo.
// The dialog shows where it goes before anything is sent; the app adds its
// version and OS. Nothing is sent without the button.

import { useEffect, useId, useRef, useState, type ChangeEvent, type FormEvent } from "react";
import { feedbackStatus, sendFeedback } from "./api";
import type { FeedbackKind, FeedbackRequest, FeedbackStatus } from "./types";

const KINDS: { id: FeedbackKind; label: string; placeholder: string }[] = [
  { id: "bug", label: "Chyba", placeholder: "Co se stalo a co se mělo stát? Kroky, jak to zopakovat, pomůžou nejvíc." },
  { id: "change", label: "Změna", placeholder: "Co by mělo fungovat jinak a proč?" },
  { id: "improvement", label: "Vylepšení", placeholder: "Co by se ti hodilo? Klidně jen nápad." },
];

/** Same limits and types as the Worker (feedback/src/feedback.ts). */
export const MAX_MESSAGE = 10_000;
export const MAX_ATTACHMENT = 2 * 1024 * 1024;
const TYPES_BY_EXTENSION: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  pdf: "application/pdf",
  txt: "text/plain",
  log: "text/plain",
  md: "text/plain",
  csv: "text/csv",
  json: "application/json",
};
const ACCEPT = Object.keys(TYPES_BY_EXTENSION)
  .map((e) => `.${e}`)
  .join(",");

type Attachment = NonNullable<FeedbackRequest["attachment"]>;
type Phase = { kind: "edit"; error: string | null } | { kind: "sending" } | { kind: "sent"; id: string };

/** File → attachment, or a sentence why not. The type comes from the extension. */
export async function readAttachment(file: File): Promise<Attachment | string> {
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  const type = TYPES_BY_EXTENSION[extension];
  if (!type) return "Tenhle typ souboru nejde přiložit. Jde obrázek, PDF, log, CSV, JSON nebo text.";
  if (file.size > MAX_ATTACHMENT) return "Příloha je větší než 2 MB.";
  const bytes = new Uint8Array(await file.arrayBuffer());
  let binary = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return { name: file.name, type, data: btoa(binary) };
}

export function FeedbackDialog({ location, onClose }: { location: string; onClose: () => void }) {
  const titleId = useId();
  const [status, setStatus] = useState<FeedbackStatus | null>(null);
  const [kind, setKind] = useState<FeedbackKind>("bug");
  const [message, setMessage] = useState("");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [attachment, setAttachment] = useState<Attachment | null>(null);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "edit", error: null });
  const messageRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    let alive = true;
    feedbackStatus().then(
      (s) => alive && setStatus(s),
      (err: unknown) =>
        alive && setStatus({ available: false, host: "", version: "", os: "", error: String(err) }),
    );
    messageRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      alive = false;
      window.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const onFile = async (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const result = await readAttachment(file);
    if (typeof result === "string") {
      setAttachment(null);
      setAttachmentError(result);
      e.target.value = "";
    } else {
      setAttachment(result);
      setAttachmentError(null);
    }
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!message.trim()) {
      setPhase({ kind: "edit", error: "Napiš, o co jde. Zpráva je prázdná." });
      messageRef.current?.focus();
      return;
    }
    setPhase({ kind: "sending" });
    const author = name.trim() || email.trim() ? { name: name.trim(), email: email.trim() } : null;
    try {
      const receipt = await sendFeedback({ kind, message, location, author, attachment });
      setPhase({ kind: "sent", id: receipt.id });
    } catch (err) {
      setPhase({ kind: "edit", error: String(err) });
    }
  };

  const current = KINDS.find((k) => k.id === kind) ?? KINDS[0]!;
  const sending = phase.kind === "sending";

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <div className="modal-head">
          <h2 id={titleId}>Zpětná vazba</h2>
          <button type="button" className="icon-close" aria-label="Zavřít" onClick={onClose}>
            ×
          </button>
        </div>

        {phase.kind === "sent" ? (
          <div className="feedback-done" role="status">
            <p className="feedback-thanks">Díky! Zpětná vazba dorazila.</p>
            <p className="hint">
              Číslo zprávy <b className="num">{phase.id}</b>. Kdyby ještě něco chybělo, pošli klidně další.
            </p>
            <div className="modal-actions">
              <button type="button" className="btn primary" onClick={onClose}>
                Zavřít
              </button>
            </div>
          </div>
        ) : (
          <form onSubmit={onSubmit} aria-label="Zpětná vazba">
            <p className="hint feedback-intro">Napiš, co nefunguje nebo co by šlo udělat líp. Každou zprávu si přečtu.</p>
            <fieldset className="field">
              <legend>Druh</legend>
              <div className="segmented">
                {KINDS.map((k) => (
                  <label key={k.id} className={kind === k.id ? "is-on" : undefined}>
                    <input type="radio" name="feedback-kind" value={k.id} checked={kind === k.id} onChange={() => setKind(k.id)} />
                    {k.label}
                  </label>
                ))}
              </div>
            </fieldset>

            <label className="field-label" htmlFor={`${titleId}-message`}>
              Zpráva
            </label>
            <textarea
              id={`${titleId}-message`}
              ref={messageRef}
              className="input textarea"
              rows={6}
              maxLength={MAX_MESSAGE}
              placeholder={current.placeholder}
              value={message}
              onChange={(e) => setMessage(e.target.value)}
            />

            <div className="feedback-who">
              <label className="field-label">
                Jméno <span className="muted">(nepovinné)</span>
                <input className="input" type="text" maxLength={100} value={name} onChange={(e) => setName(e.target.value)} />
              </label>
              <label className="field-label">
                E-mail <span className="muted">(nepovinný)</span>
                <input className="input" type="email" maxLength={200} value={email} onChange={(e) => setEmail(e.target.value)} />
              </label>
            </div>
            <p className="hint">Bez jména a e-mailu se zpráva pošle anonymně.</p>

            <label className="field-label">
              Příloha <span className="muted">(nepovinná: obrázek, PDF, log, CSV, JSON nebo text do 2 MB)</span>
              <input className="file" type="file" accept={ACCEPT} onChange={(e) => void onFile(e)} />
            </label>
            {attachmentError && <p className="status err">{attachmentError}</p>}

            {status && !status.available && <p className="status err">Odeslat teď nejde: {status.error}</p>}

            {phase.kind === "edit" && phase.error && (
              <p className="status err" role="alert">
                {phase.error}
              </p>
            )}

            <div className="modal-actions">
              <button type="button" className="btn" onClick={onClose} disabled={sending}>
                Zrušit
              </button>
              <button type="submit" className="btn primary" disabled={sending || !status?.available}>
                {sending ? "Odesílám…" : "Odeslat"}
              </button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}
