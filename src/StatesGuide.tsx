// "Stavy" tab: every state the widget and the tray can show, what it means and
// when it happens. The mood rules mirror crates/usage-core/src/mood.rs and the
// limit rules codex.rs / t3.rs; change them together.

import { useEffect, useState, type CSSProperties, type FocusEvent, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { Character } from "./Character";
import { CREATURE_NAME, MOOD_CAPTION, MOOD_LABEL, PROVIDER_NAME } from "./format";
import type { Mood, Provider, Snapshot } from "./types";

const PROVIDERS: Provider[] = ["claude", "codex"];

const capitalize = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/** Size of the hover preview (app.css .mood-preview); used to keep it on screen. */
const PREVIEW_W = 480;
const PREVIEW_H = 280;
const GAP = 14;

interface Preview {
  mood: Mood;
  left: number;
  top: number;
}

/** Next to the thumbnails, vertically centred on them, always inside the window. */
function placePreview(anchor: DOMRect): { left: number; top: number } {
  const maxLeft = Math.max(8, window.innerWidth - PREVIEW_W - 8);
  const maxTop = Math.max(8, window.innerHeight - PREVIEW_H - 8);
  return {
    left: Math.min(Math.max(8, anchor.right + GAP), maxLeft),
    top: Math.min(Math.max(8, anchor.top + anchor.height / 2 - PREVIEW_H / 2), maxTop),
  };
}

/** Large animated preview of one mood: both creatures, their captions. */
function MoodPreview({ preview }: { preview: Preview }) {
  const style = { left: `${preview.left}px`, top: `${preview.top}px` } as CSSProperties;
  return createPortal(
    <div className="mood-preview" role="tooltip" id="mood-preview" style={style}>
      <div className="mood-preview-title">{capitalize(MOOD_LABEL[preview.mood])}</div>
      <div className="mood-preview-stage">
        {PROVIDERS.map((p) => (
          <figure key={p} className={`mood-preview-slot slot-${p}`}>
            <Character provider={p} mood={preview.mood} />
            <figcaption>
              <b>{CREATURE_NAME[p]}</b> {MOOD_CAPTION[p][preview.mood]}
            </figcaption>
          </figure>
        ))}
      </div>
    </div>,
    document.body,
  );
}

/** Priority order: when several apply, the higher one wins. */
const MOODS: { mood: Mood; when: string }[] = [
  {
    mood: "confused",
    when: "Nevím, kolik limitu zbývá: limity se nedaří načíst (Codex CLI chybí nebo neodpovídá, Claude Code ani T3 Code zatím limity Claude nenahlásily), nebo se změnil formát záznamů Claude Code.",
  },
  {
    mood: "dozing",
    when: "Znám jen starší číslo. Codex: poslední čtení je starší než 10 minut. Claude: po posledním hlášení z Claude Code nebo T3 Code běžel Claude jinde (třeba v Claude Desktop), takže limit může být vyšší.",
  },
  { mood: "panic", when: "Limit 5h nebo týdenní je na 95 % a víc. Uklidní se, až klesne pod 90 %." },
  { mood: "sweat", when: "Limit 5h nebo týdenní je na 75 % a víc. Uklidní se, až klesne pod 70 %." },
  {
    mood: "celebrate",
    when: "Čtyři sekundy po novém milníku: násobek paušálu 1×, 2×, 3×, 5× a 10×, denní rekord, série 7, 14 a 30 dní za sebou.",
  },
  { mood: "focus", when: "Pracuje se: nová spotřeba v posledních 5 minutách." },
  { mood: "sleep", when: "30 minut bez práce." },
  {
    mood: "calm",
    when: "Nic z toho výše: limity v pořádku a poslední práce před 5 až 30 minutami (nebo aktivitu zrovna nejde sledovat).",
  },
];

const LIMIT_STATES: { value: string; sub: string; tone: string; title: string; text: string }[] = [
  {
    value: "42 %",
    sub: "reset za 2 h 10 min",
    tone: "ok",
    title: "Známý stav",
    text: "Čerstvé číslo. Pruh je zelený pod 75 %, oranžový od 75 % a červený od 95 %.",
  },
  {
    value: "~42 %",
    sub: "poslední známá · před 20 min",
    tone: "stale",
    title: "Zastaralé",
    text: "Šrafovaný pruh. Codex neodpověděl přes 10 minut, nebo Claude běžel tam, odkud limity nechodí. Skutečné číslo může být vyšší.",
  },
  {
    value: "0 %",
    sub: "začne první zprávou",
    tone: "empty",
    title: "Nové okno",
    text: "Okno 5h Claude se po resetu otevře až s další zprávou. Do té doby se nic nečerpá.",
  },
  {
    value: "?",
    sub: "po resetu, aktuální stav neznámý",
    tone: "unknown",
    title: "Po resetu",
    text: "Okno se mezitím vynulovalo a od té doby se pracovalo, ale nové číslo ještě nepřišlo.",
  },
  {
    value: "?",
    sub: "stav neznámý",
    tone: "unknown",
    title: "Neznámé",
    text: "Číslo se zatím nepodařilo zjistit vůbec. Poznámka pod řádkem říká proč.",
  },
  {
    value: "–",
    sub: "limity Claude: připoj Claude Code v Nastavení",
    tone: "off",
    title: "Bez zdroje",
    text: "Claude Code limity nehlásí (není přihlášený ani připojený) a T3 Code tu není, takže limity Claude nejde zjistit. Postavička pak reaguje jen na skóre a aktivitu.",
  },
];

const SOURCES: { title: string; text: string }[] = [
  {
    title: "Limity Codexu",
    text: "Oficiální codex app-server, dotaz každé 2 minuty. Platí pro celý účet ChatGPT, tedy i pro práci v T3 Code a v terminálu.",
  },
  {
    title: "Limity Claude",
    text: "Tři zdroje, platí novější číslo. Claude Code si limity sám načítá zhruba každých 10 minut a ukládá do ~/.claude.json. Claude Code v terminálu (iTerm2, Terminál…) je navíc předává svému stavovému řádku po každé odpovědi, stačí ho připojit v Nastavení. T3 Code si zapisuje události limitů z oficiálního Claude Agent SDK do svých logů (~/.t3/userdata/logs/provider). Přihlašovací token Claude aplikace nikdy nečte. Claude Desktop limity nehlásí, proto číslo po práci v něm zešedne na ~.",
  },
  {
    title: "Limit Fable",
    text: "Řádek Claude · Fable pod týdnem Claude: týdenní limit jen pro Fable, pokud ho tvůj tarif má. Bere se ze stejné mezipaměti Claude Code a zešedne jen po práci s Fable, ne s jinými modely.",
  },
  {
    title: "Skóre Claude",
    text: "Lokální záznamy Claude Code (~/.claude/projects) přepočtené ceníkem API (snímek LiteLLM) a kurzem ČNB.",
  },
  {
    title: "Skóre Codexu",
    text: "Denní součty tokenů z Codexu krát průměrná sazba 5,70 USD za milion tokenů. Rozpad podle modelu Codex nevrací, proto je to odhad.",
  },
];

const SCORE_BADGES: { badge: string; text: string }[] = [
  { badge: "≈ odhad", text: "Codex: denní tokeny krát průměrná sazba. Vypnout jde v Nastavení › Skóre." },
  { badge: "částečně", text: "Některé modely nemají v ceníku cenu. Jejich tokeny se nepočítají (nikdy jako 0 €)." },
  { badge: "?", text: "Část skóre je nejistá: zdroj chybí, nebo se změnil formát záznamů." },
  { badge: "nezadáno", text: "Paušál není vyplněný, takže se nepočítá násobek. Doplň ho v Nastavení › Paušál." },
];

const TRAY: { cls: string; title: string; text: string }[] = [
  { cls: "green", title: "Zelená", text: "Nejvyšší limit je pod 75 %." },
  { cls: "orange", title: "Oranžová", text: "Některý limit je na 75 % a víc (zelená se vrátí pod 70 %)." },
  { cls: "red", title: "Červená", text: "Některý limit je na 95 % a víc (oranžová se vrátí pod 90 %)." },
  { cls: "dim", title: "Tlumená", text: "Barva podle posledního známého, ale zastaralého čísla." },
  { cls: "grey", title: "Šedá", text: "Žádné číslo limitu zatím není." },
  { cls: "mono", title: "Jednobarevná", text: "Limity nejsou k dispozici u Claude ani u Codexu." },
];

export function StatesGuide({ snapshot }: { snapshot: Snapshot | null }) {
  const current = (p: Provider): Mood | undefined => snapshot?.moods.find((m) => m.provider === p)?.mood;
  const [preview, setPreview] = useState<Preview | null>(null);

  // A fixed-position preview would drift away from its row while scrolling.
  useEffect(() => {
    if (!preview) return;
    const close = () => setPreview(null);
    window.addEventListener("scroll", close, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("resize", close);
    };
  }, [preview]);

  const show = (mood: Mood) => (e: MouseEvent<HTMLElement> | FocusEvent<HTMLElement>) =>
    setPreview({ mood, ...placePreview(e.currentTarget.getBoundingClientRect()) });
  const hide = () => setPreview(null);

  return (
    <div className="guide">
      <section className="card guide-moods" aria-labelledby="guide-moods">
        <h2 id="guide-moods">Nálady postaviček</h2>
        <p className="hint">
          Sunnie hlídá Claude, Dropie hlídá Codex. Když platí víc stavů najednou, vyhraje ten, který je v seznamu výš. Najeď myší na obrázek a uvidíš velký náhled i s animací.
        </p>
        <ol className="mood-list">
          {MOODS.map(({ mood, when }) => {
            const now = PROVIDERS.filter((p) => current(p) === mood);
            return (
              <li key={mood} className={now.length ? "mood-item is-now" : "mood-item"}>
                <div
                  className={preview?.mood === mood ? "mood-pics is-previewed" : "mood-pics"}
                  tabIndex={0}
                  role="button"
                  aria-label={`Velký náhled: ${MOOD_LABEL[mood]}`}
                  aria-describedby={preview?.mood === mood ? "mood-preview" : undefined}
                  onMouseEnter={show(mood)}
                  onMouseLeave={hide}
                  onFocus={show(mood)}
                  onBlur={hide}
                  onKeyDown={(e) => e.key === "Escape" && hide()}
                  // Touch screens have no hover: a tap toggles the preview.
                  onClick={(e) => (preview?.mood === mood ? hide() : show(mood)(e))}
                >
                  {PROVIDERS.map((p) => (
                    <Character key={p} provider={p} mood={mood} />
                  ))}
                </div>
                <div className="mood-text">
                  <h3>
                    {capitalize(MOOD_LABEL[mood])}
                    {now.map((p) => (
                      <span key={p} className={`now-tag now-${p}`}>
                        teď {CREATURE_NAME[p]}
                      </span>
                    ))}
                  </h3>
                  <p className="mood-captions">
                    {PROVIDERS.map((p) => `${CREATURE_NAME[p]} ${MOOD_CAPTION[p][mood]}`).join(" · ")}
                  </p>
                  <p className="mood-when">{when}</p>
                </div>
              </li>
            );
          })}
        </ol>
      </section>

      <div className="guide-side">
        <section className="card" aria-labelledby="guide-limits">
          <h2 id="guide-limits">Řádky limitů</h2>
          <ul className="state-list">
            {LIMIT_STATES.map((s) => (
              <li key={s.title}>
                <div className="state-sample">
                  <span className="sample-sub">{s.sub}</span>
                  <span className="sample-value num">{s.value}</span>
                  <span className={`sample-bar sample-${s.tone}`} aria-hidden="true" />
                </div>
                <p>
                  <b>{s.title}.</b> {s.text}
                </p>
              </li>
            ))}
          </ul>
        </section>

        <section className="card" aria-labelledby="guide-sources">
          <h2 id="guide-sources">Odkud čísla jsou</h2>
          <dl className="def-list">
            {SOURCES.map((s) => (
              <div key={s.title}>
                <dt>{s.title}</dt>
                <dd>{s.text}</dd>
              </div>
            ))}
          </dl>
        </section>

        <section className="card" aria-labelledby="guide-score">
          <h2 id="guide-score">Značky u skóre</h2>
          <dl className="def-list badges">
            {SCORE_BADGES.map((s) => (
              <div key={s.badge}>
                <dt>
                  <span className="tag">{s.badge}</span>
                </dt>
                <dd>{s.text}</dd>
              </div>
            ))}
          </dl>
        </section>

        <section className="card" aria-labelledby="guide-tray">
          <h2 id="guide-tray">Ikona v oznamovací oblasti</h2>
          <ul className="tray-list">
            {TRAY.map((t) => (
              <li key={t.cls}>
                <i className={`swatch swatch-${t.cls}`} aria-hidden="true" />
                <b>{t.title}</b>
                <span>{t.text}</span>
              </li>
            ))}
          </ul>
          <p className="hint tray-note">
            Počítá se nejvyšší limit z {PROVIDER_NAME.claude} i {PROVIDER_NAME.codex}.
          </p>
        </section>
      </div>

      {preview && <MoodPreview key={preview.mood} preview={preview} />}
    </div>
  );
}
