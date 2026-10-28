// Browser-only gallery of every mood for both mascots (`?window=gallery`).

import { Character } from "../Character";
import { CREATURE_NAME, MOOD_LABEL } from "../format";
import type { Mood, Provider } from "../types";
import "../styles/dev.css";

const MOODS: Mood[] = ["calm", "focus", "sweat", "panic", "celebrate", "dozing", "sleep", "confused"];
const PROVIDERS: Provider[] = ["claude", "codex"];

export default function MoodGallery() {
  return (
    <main className="gallery">
      <h1>Nálady postaviček</h1>
      {PROVIDERS.map((p) => (
        <section key={p}>
          <h2>{CREATURE_NAME[p]}</h2>
          <div className="gallery-row">
            {MOODS.map((m) => (
              <figure key={m}>
                <Character provider={p} mood={m} />
                <figcaption>
                  {MOOD_LABEL[m]} <code>mood-{m}</code>
                </figcaption>
              </figure>
            ))}
          </div>
        </section>
      ))}
    </main>
  );
}
