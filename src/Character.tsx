// Two hand-drawn SVG mascots. The SVG only describes shapes; every mood is a
// class on the root (`mood-<name>`) and character.css decides which face parts
// are visible and how they move (transform/opacity keyframes only).
//
// Rule for future edits: never animate an element that also carries an SVG
// `transform` attribute (CSS transform would replace it). Wrap it in a <g>.

import { useId } from "react";
import { CREATURE_NAME, MOOD_LABEL } from "./format";
import type { Mood, Provider } from "./types";

export interface CharacterProps {
  provider: Provider;
  mood: Mood;
  className?: string;
}

export function Character({ provider, mood, className }: CharacterProps) {
  const label = `${CREATURE_NAME[provider]}: ${MOOD_LABEL[mood]}`;
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const cls = ["mascot", `mascot-${provider}`, `mood-${mood}`, className].filter(Boolean).join(" ");
  return (
    <svg className={cls} viewBox="0 0 140 124" role="img" aria-label={label} focusable="false">
      <title>{label}</title>
      {provider === "claude" ? <ClaudeCreature uid={uid} /> : <CodexCreature uid={uid} />}
    </svg>
  );
}

const INK = "#2A1E1C";
const CHEEK = "#FF7B86";
const TONGUE = "#FF8E8E";

interface FaceProps {
  cx: number;
  eyeY: number;
  gap: number;
  mouthY: number;
  cheekY: number;
  cheekGap: number;
}

/** Every face variant is drawn once; character.css shows the ones for the mood. */
function Face({ cx, eyeY, gap, mouthY: my, cheekY, cheekGap }: FaceProps) {
  const lx = cx - gap;
  const rx = cx + gap;
  return (
    <g className="face">
      <g className="cheeks" fill={CHEEK}>
        <ellipse cx={cx - cheekGap} cy={cheekY} rx="4.6" ry="2.6" />
        <ellipse cx={cx + cheekGap} cy={cheekY} rx="4.6" ry="2.6" />
      </g>

      {/* eyes */}
      <g className="v eyes-open">
        <g className="blink" fill={INK}>
          <ellipse cx={lx} cy={eyeY} rx="4" ry="5" />
          <ellipse cx={rx} cy={eyeY} rx="4" ry="5" />
          <circle cx={lx + 1.4} cy={eyeY - 1.9} r="1.35" fill="#FFFFFF" />
          <circle cx={rx + 1.4} cy={eyeY - 1.9} r="1.35" fill="#FFFFFF" />
        </g>
      </g>
      <g className="v eyes-wide">
        <circle cx={lx} cy={eyeY} r="6.2" fill="#FFF8F2" stroke={INK} strokeWidth="1.1" />
        <circle cx={rx} cy={eyeY} r="6.2" fill="#FFF8F2" stroke={INK} strokeWidth="1.1" />
        <g className="pupils" fill={INK}>
          <circle cx={lx} cy={eyeY + 0.4} r="2.3" />
          <circle cx={rx} cy={eyeY + 0.4} r="2.3" />
        </g>
      </g>
      <g className="v eyes-half">
        <g fill={INK}>
          <path d={`M${lx - 4} ${eyeY + 0.6} A4 3.8 0 0 0 ${lx + 4} ${eyeY + 0.6} Z`} />
          <path d={`M${rx - 4} ${eyeY + 0.6} A4 3.8 0 0 0 ${rx + 4} ${eyeY + 0.6} Z`} />
          <circle cx={lx + 1.5} cy={eyeY + 2} r="0.9" fill="#FFFFFF" />
          <circle cx={rx + 1.5} cy={eyeY + 2} r="0.9" fill="#FFFFFF" />
        </g>
        {/* droopy lids: a soft arch, ends slightly lower than the middle */}
        <path
          d={`M${lx - 5} ${eyeY + 1.2} Q${lx} ${eyeY - 1.6} ${lx + 5} ${eyeY + 1.2} M${rx - 5} ${eyeY + 1.2} Q${rx} ${eyeY - 1.6} ${rx + 5} ${eyeY + 1.2}`}
          fill="none"
          stroke={INK}
          strokeWidth="1.8"
          strokeLinecap="round"
        />
      </g>
      <g className="v eyes-closed" fill="none" stroke={INK} strokeWidth="2" strokeLinecap="round">
        <path d={`M${lx - 4.5} ${eyeY} Q${lx} ${eyeY + 4} ${lx + 4.5} ${eyeY}`} />
        <path d={`M${rx - 4.5} ${eyeY} Q${rx} ${eyeY + 4} ${rx + 4.5} ${eyeY}`} />
      </g>
      <g className="v eyes-happy" fill="none" stroke={INK} strokeWidth="2.2" strokeLinecap="round">
        <path d={`M${lx - 4.5} ${eyeY + 1.5} Q${lx} ${eyeY - 4.5} ${lx + 4.5} ${eyeY + 1.5}`} />
        <path d={`M${rx - 4.5} ${eyeY + 1.5} Q${rx} ${eyeY - 4.5} ${rx + 4.5} ${eyeY + 1.5}`} />
      </g>

      {/* brows */}
      <g className="v brows-worried" fill="none" stroke={INK} strokeWidth="1.8" strokeLinecap="round">
        <path d={`M${lx - 4.5} ${eyeY - 8} L${lx + 3.5} ${eyeY - 10.6}`} />
        <path d={`M${rx + 4.5} ${eyeY - 8} L${rx - 3.5} ${eyeY - 10.6}`} />
      </g>
      <g className="v brows-confused" fill="none" stroke={INK} strokeWidth="1.8" strokeLinecap="round">
        <path d={`M${lx - 4} ${eyeY - 8.2} L${lx + 4} ${eyeY - 8.6}`} />
        <path d={`M${rx - 4.5} ${eyeY - 9.5} Q${rx} ${eyeY - 14} ${rx + 4.5} ${eyeY - 10.5}`} />
      </g>

      {/* mouths */}
      <path
        className="v mouth-smile"
        d={`M${cx - 5} ${my} Q${cx} ${my + 4.6} ${cx + 5} ${my}`}
        fill="none"
        stroke={INK}
        strokeWidth="2"
        strokeLinecap="round"
      />
      <g className="v mouth-focus">
        <path d={`M${cx + 0.6} ${my + 1} Q${cx + 2.2} ${my + 5} ${cx + 4.2} ${my + 1.2} Z`} fill={TONGUE} />
        <path d={`M${cx - 4} ${my + 1} H${cx + 4}`} stroke={INK} strokeWidth="2" strokeLinecap="round" />
      </g>
      <path
        className="v mouth-wobble"
        d={`M${cx - 6} ${my + 1.5} q1.5 -2.2 3 0 t3 0 t3 0 t3 0`}
        fill="none"
        stroke={INK}
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <ellipse className="v mouth-o" cx={cx} cy={my + 2} rx="3.1" ry="3.9" fill={INK} />
      <g className="v mouth-grin">
        <path d={`M${cx - 6.5} ${my - 0.5} Q${cx} ${my + 9.5} ${cx + 6.5} ${my - 0.5} Z`} fill={INK} />
        <ellipse cx={cx} cy={my + 3.6} rx="2.8" ry="1.5" fill={TONGUE} />
      </g>
      <path
        className="v mouth-flat"
        d={`M${cx - 3.5} ${my + 1.2} H${cx + 3.5}`}
        stroke={INK}
        strokeWidth="1.9"
        strokeLinecap="round"
      />
      <ellipse className="v mouth-sleep" cx={cx} cy={my + 1.6} rx="1.8" ry="2.2" fill={INK} />
      <path
        className="v mouth-squiggle"
        d={`M${cx - 5} ${my + 1.5} q2.5 -3 5 0 t5 0`}
        fill="none"
        stroke={INK}
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </g>
  );
}

interface ExtrasProps {
  sweat: [number, number][];
  zzz: [number, number];
  question: [number, number];
  sparkles: [number, number][];
}

/** Mood props around the head: sweat drops, zzz, "?" and sparkles. */
function Extras({ sweat, zzz, question, sparkles }: ExtrasProps) {
  const [zx, zy] = zzz;
  return (
    <>
      <g className="v sweat" fill="#9ED8FF" stroke="#5FB4EA" strokeWidth="0.6">
        {sweat.map(([x, y], i) => (
          <path key={i} className={`drop drop-${i + 1}`} d={`M${x} ${y} q4.2 6.2 0 8.8 q-4.2 -2.6 0 -8.8 Z`} />
        ))}
      </g>
      <g className="v zzz" fill="#CFD8EA" fontFamily="Segoe UI Variable Display, Segoe UI, system-ui, sans-serif" fontWeight="700">
        <text className="z z1" x={zx} y={zy} fontSize="9">z</text>
        <text className="z z2" x={zx + 8} y={zy - 9} fontSize="11">z</text>
        <text className="z z3" x={zx + 17} y={zy - 19} fontSize="13">z</text>
      </g>
      <text
        className="v question"
        x={question[0]}
        y={question[1]}
        fontSize="21"
        fontWeight="800"
        fill="#FFE08A"
        fontFamily="Segoe UI Variable Display, Segoe UI, system-ui, sans-serif"
      >
        ?
      </text>
      <g className="v sparkles" fill="#FFE08A">
        {sparkles.map(([x, y], i) => (
          <g key={i} transform={`translate(${x} ${y})`}>
            <path className={`sparkle sparkle-${i + 1}`} d="M0 -6.5 L1.6 -1.6 L6.5 0 L1.6 1.6 L0 6.5 L-1.6 1.6 L-6.5 0 L-1.6 -1.6 Z" />
          </g>
        ))}
      </g>
    </>
  );
}

/** Long and short tapered rays, pointing up from the origin. */
const RAY_LONG = "M-6 -22 L-2.3 -44 Q0 -47.5 2.3 -44 L6 -22 Z";
const RAY_SHORT = "M-6 -22 L-2.4 -39.5 Q0 -43 2.4 -39.5 L6 -22 Z";

function ClaudeCreature({ uid }: { uid: string }) {
  const body = `${uid}-claude-body`;
  return (
    <>
      <defs>
        <radialGradient id={body} cx="40%" cy="34%" r="70%">
          <stop offset="0" stopColor="#FFB892" />
          <stop offset="0.55" stopColor="#F28B61" />
          <stop offset="1" stopColor="#D9683F" />
        </radialGradient>
      </defs>

      <ellipse className="shadow" cx="70" cy="115" rx="36" ry="4.5" fill="#000000" opacity="0.28" />

      <g className="creature">
        <g className="body">
          <g transform="translate(70 60)" fill="#E5754B" stroke="#E5754B" strokeWidth="2" strokeLinejoin="round">
            {Array.from({ length: 12 }, (_, i) => (
              <path key={i} d={i % 2 === 0 ? RAY_LONG : RAY_SHORT} transform={`rotate(${i * 30 + 15})`} />
            ))}
          </g>
          <circle cx="70" cy="60" r="32" fill={`url(#${body})`} />
          <ellipse cx="58" cy="43" rx="10" ry="5.5" fill="#FFFFFF" opacity="0.2" transform="rotate(-24 58 43)" />
          <Face cx={70} eyeY={55} gap={10.5} mouthY={64} cheekY={63.5} cheekGap={18} />
        </g>

        <g className="coins" fill="#FFD166" stroke="#D99A2B" strokeWidth="0.8">
          <circle className="coin coin-1" cx="58" cy="82" r="3.3" />
          <circle className="coin coin-2" cx="70" cy="80" r="3.3" />
          <circle className="coin coin-3" cx="82" cy="82" r="3.3" />
        </g>

        <g className="abacus">
          <rect x="43" y="80" width="54" height="28" rx="5" fill="#8C5B3D" />
          <rect x="43" y="80" width="54" height="3.2" rx="1.6" fill="#A87252" />
          <rect x="47.5" y="84" width="45" height="20" rx="2.5" fill="#2C211E" />
          <g stroke="#C9A27D" strokeWidth="0.9" opacity="0.85">
            <path d="M47.5 89 H92.5 M47.5 94 H92.5 M47.5 99 H92.5" />
          </g>
          <g className="beads beads-1" fill="#FFC857">
            <ellipse cx="52" cy="89" rx="2.7" ry="2.3" />
            <ellipse cx="57.6" cy="89" rx="2.7" ry="2.3" />
            <ellipse cx="63.2" cy="89" rx="2.7" ry="2.3" />
          </g>
          <g className="beads beads-2" fill="#FFF1DE">
            <ellipse cx="52" cy="94" rx="2.7" ry="2.3" />
            <ellipse cx="57.6" cy="94" rx="2.7" ry="2.3" />
          </g>
          <g className="beads beads-3" fill="#FF9F7A">
            <ellipse cx="52" cy="99" rx="2.7" ry="2.3" />
            <ellipse cx="57.6" cy="99" rx="2.7" ry="2.3" />
            <ellipse cx="63.2" cy="99" rx="2.7" ry="2.3" />
            <ellipse cx="68.8" cy="99" rx="2.7" ry="2.3" />
          </g>
        </g>

        <g className="arm arm-l">
          <path d="M44 70 Q38 80 47 87" fill="none" stroke="#E5754B" strokeWidth="5.5" strokeLinecap="round" />
          <circle cx="47.5" cy="87" r="4.3" fill="#F49A73" stroke="#D2633B" strokeWidth="0.8" />
        </g>
        <g className="arm arm-r">
          <path d="M96 70 Q102 80 93 87" fill="none" stroke="#E5754B" strokeWidth="5.5" strokeLinecap="round" />
          <circle cx="92.5" cy="87" r="4.3" fill="#F49A73" stroke="#D2633B" strokeWidth="0.8" />
        </g>
      </g>

      <Extras
        sweat={[
          [104, 36],
          [35, 42],
        ]}
        zzz={[103, 30]}
        question={[104, 27]}
        sparkles={[
          [18, 28],
          [122, 22],
          [12, 78],
          [128, 70],
        ]}
      />
    </>
  );
}

function CodexCreature({ uid }: { uid: string }) {
  const body = `${uid}-codex-body`;
  const screen = `${uid}-codex-screen`;
  return (
    <>
      <defs>
        <linearGradient id={body} x1="0.2" y1="0" x2="0.75" y2="1">
          <stop offset="0" stopColor="#97EFDA" />
          <stop offset="0.55" stopColor="#5CCDB4" />
          <stop offset="1" stopColor="#34A88F" />
        </linearGradient>
        <linearGradient id={screen} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#141B24" />
          <stop offset="1" stopColor="#0C1117" />
        </linearGradient>
      </defs>

      <ellipse className="shadow" cx="70" cy="117" rx="54" ry="3.6" fill="#000000" opacity="0.26" />

      {/* desk legs sit behind the creature */}
      <g fill="#343C4B">
        <rect x="16" y="99" width="4" height="16" rx="1.5" />
        <rect x="120" y="99" width="4" height="16" rx="1.5" />
      </g>

      <g className="creature">
        <g className="body">
          <g className="antenna">
            <path d="M56 27 Q54.5 17 61 12" fill="none" stroke="#3DB39A" strokeWidth="2.4" strokeLinecap="round" />
            <circle className="antenna-bulb" cx="62" cy="11" r="3.4" fill="#B6F5E6" />
          </g>
          <path
            d="M56 26 C79 26 90 41 90 63 C90 84 78 95 56 95 C34 95 22 84 22 63 C22 41 33 26 56 26 Z"
            fill={`url(#${body})`}
          />
          <ellipse cx="43" cy="40" rx="9" ry="5" fill="#FFFFFF" opacity="0.22" transform="rotate(-28 43 40)" />
          <Face cx={56} eyeY={56} gap={10} mouthY={66} cheekY={64.5} cheekGap={17} />
        </g>
      </g>

      {/* desk top and everything on it stays put while the creature moves */}
      <rect x="8" y="94" width="124" height="6" rx="3" fill="#475165" />
      <rect x="8" y="94" width="124" height="1.6" rx="0.8" fill="#5B667C" />

      <g className="mug">
        <g className="steam" fill="none" stroke="#CFD8EA" strokeWidth="1.2" strokeLinecap="round">
          <path className="steam-1" d="M15 80 q-2 -3 0 -6 t0 -6" />
          <path className="steam-2" d="M19.5 79 q-2 -3 0 -6 t0 -6" />
        </g>
        <path d="M22 85.5 q4.2 0 4.2 3.3 q0 3.3 -4.2 3.3" fill="none" stroke="#EDE3D6" strokeWidth="1.9" />
        <rect x="11.5" y="82.5" width="11" height="11.5" rx="2.2" fill="#EDE3D6" />
        <ellipse cx="17" cy="83.4" rx="4.6" ry="1.1" fill="#6B4A3A" />
      </g>

      <rect x="37" y="89.5" width="38" height="4.5" rx="1.6" fill="#2A303C" stroke="#5A6476" strokeWidth="0.6" />

      <g className="creature-hands">
        <ellipse className="hand hand-l" cx="46" cy="89.5" rx="5" ry="3.6" fill="#55C8AE" stroke="#2F9C84" strokeWidth="0.8" />
        <ellipse className="hand hand-r" cx="66" cy="89.5" rx="5" ry="3.6" fill="#55C8AE" stroke="#2F9C84" strokeWidth="0.8" />
      </g>

      <g className="terminal">
        <rect x="104" y="85.5" width="6" height="6" fill="#4A5466" />
        <rect x="98" y="91.5" width="18" height="3" rx="1.5" fill="#4A5466" />
        <rect x="86" y="56" width="42" height="30" rx="4" fill="#262C38" stroke="#566072" strokeWidth="1.2" />
        <rect x="89.5" y="59.5" width="35" height="23" rx="2" fill={`url(#${screen})`} />
        <g className="screen">
          <path d="M93 63.6 l3 2.4 l-3 2.4" fill="none" stroke="#6FE0C6" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
          <g fill="#6FE0C6" opacity="0.7">
            <rect className="term-line term-line-1" x="99" y="65" width="16" height="2" rx="1" />
            <rect className="term-line term-line-2" x="93" y="70" width="24" height="2" rx="1" />
            <rect className="term-line term-line-3" x="93" y="75" width="12" height="2" rx="1" />
          </g>
          <rect className="cursor" x="107" y="74.5" width="4.5" height="3" rx="0.5" fill="#B6F5E6" />
        </g>
        <rect className="screen-flash" x="89.5" y="59.5" width="35" height="23" rx="2" fill="#E5675C" />
        <rect className="screen-dim" x="89.5" y="59.5" width="35" height="23" rx="2" fill="#0B0F14" />
      </g>

      <Extras
        sweat={[
          [90, 32],
          [21, 40],
        ]}
        zzz={[84, 27]}
        question={[82, 26]}
        sparkles={[
          [12, 22],
          [108, 16],
          [8, 66],
          [134, 44],
        ]}
      />
    </>
  );
}
