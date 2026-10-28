#!/usr/bin/env node
/**
 * CHANGELOG.md a čísla verzí podle Semantic Versioning 2.0.0-rc.2
 * (https://semver.org/spec/v2.0.0-rc.2.html).
 *
 * Používá to vydávací workflow (text sekce k verzi se stane popisem vydání
 * na GitHubu a poznámkami v okně aktualizace) a testy, které hlídají, že
 * CHANGELOG i čísla verzí v projektu drží pravidla.
 *
 * Když sekce k verzi chybí nebo je prázdná, skončí to chybou. To je záměr:
 * vydání bez poznámek se nemá sestavit.
 *
 * Je to obyčejný JavaScript, ne TypeScript v `src/`, protože to musí umět
 * spustit Node přímo v CI, kde se nic nepřekládá.
 */

import { readFileSync } from 'node:fs'

/**
 * Platná verze podle SemVer 2.0.0-rc.2: X.Y.Z bez úvodních nul (pravidlo 2),
 * volitelně předběžná verze za pomlčkou (pravidlo 9) a metadata sestavení
 * za plusem (pravidlo 10). Identifikátory jsou [0-9A-Za-z-].
 */
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/
const VERSION_IN_HEADING = /^##\s+\[?v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)\]?/

export function isValidVersion(version) {
  return VERSION.test(version)
}

/** Rozložená verze, nebo `null`, když neplatí. */
export function parseVersion(version) {
  const m = VERSION.exec(version)
  if (!m) return null
  return {
    major: Number(m[1]),
    minor: Number(m[2]),
    patch: Number(m[3]),
    pre: m[4] ? m[4].split('.') : [],
    build: m[5] ? m[5].split('.') : [],
  }
}

/**
 * Přednost podle pravidla 11: major, minor a patch číselně; verze
 * s předběžným označením má nižší přednost než normální verze; předběžná
 * označení se porovnávají po identifikátorech (číselné číselně, ostatní podle
 * ASCII, číselné mají nižší přednost). Metadata sestavení se nepočítají
 * (pravidlo 10). Vrací záporné číslo, nulu, nebo kladné číslo.
 */
export function compareVersions(a, b) {
  const x = parseVersion(a)
  const y = parseVersion(b)
  if (!x || !y) throw new Error(`Neplatná verze: ${!x ? a : b}`)
  for (const key of ['major', 'minor', 'patch']) {
    if (x[key] !== y[key]) return x[key] - y[key]
  }
  if (x.pre.length === 0 || y.pre.length === 0) return y.pre.length - x.pre.length
  const numeric = (s) => /^\d+$/.test(s)
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i]
    const q = y.pre[i]
    if (p === undefined) return -1
    if (q === undefined) return 1
    if (p === q) continue
    if (numeric(p) && numeric(q)) return Number(p) - Number(q)
    if (numeric(p)) return -1
    if (numeric(q)) return 1
    return p < q ? -1 : 1
  }
  return 0
}

/** Nadpis sekce: `## [1.2.3] — 2026-10-28`, `## 1.2.3`, `## v1.2.3 – cokoliv`. */
function headingFor(version) {
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`^##\\s+\\[?v?${escaped}\\]?\\s*(?:[—–-].*)?$`)
}

/**
 * Text sekce k dané verzi, nebo `null`, když tam není.
 *
 * Bere všechno až po další nadpis druhé úrovně. Samotný nadpis se nevrací;
 * v popisu vydání i v okně aktualizace je číslo verze vidět jinde.
 */
export function extractRelease(markdown, version) {
  const lines = markdown.split(/\r?\n/)
  const heading = headingFor(version)

  const start = lines.findIndex((line) => heading.test(line))
  if (start === -1) return null

  const rest = lines.slice(start + 1)
  const end = rest.findIndex((line) => /^##\s/.test(line))
  const body = (end === -1 ? rest : rest.slice(0, end))
    // Odkazy pod čarou (`[0.6.0]: https://…`) do textu vydání nepatří.
    .filter((line) => !/^\[[^\]]+\]:\s+\S/.test(line))
    .join('\n')
    .trim()

  return body === '' ? null : body
}

/** Verze, ke kterým je sekce, v pořadí, v jakém jsou v souboru. */
export function listVersions(markdown) {
  return markdown
    .split(/\r?\n/)
    .map((line) => VERSION_IN_HEADING.exec(line)?.[1])
    .filter((value) => value !== undefined)
}

/** Datum vydání z nadpisu (`— 2026-10-28`), nebo `null`. */
export function releaseDate(markdown, version) {
  const line = markdown.split(/\r?\n/).find((l) => headingFor(version).test(l))
  return line ? (/(\d{4}-\d{2}-\d{2})/.exec(line)?.[1] ?? null) : null
}

// --- spuštění z příkazové řádky --------------------------------------------

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, '/'))

if (isMain) {
  const version = process.argv[2]
  if (!version) {
    console.error('Použití: node scripts/changelog.mjs <verze>   (například 0.6.0)')
    process.exit(2)
  }
  if (!isValidVersion(version)) {
    console.error(`${version} není platná verze podle SemVer 2.0.0-rc.2 (X.Y.Z, volitelně -předběžná+sestavení).`)
    process.exit(2)
  }

  const markdown = readFileSync(new URL('../CHANGELOG.md', import.meta.url), 'utf8')
  const notes = extractRelease(markdown, version)

  if (notes === null) {
    console.error(
      `V CHANGELOG.md není co říct k verzi ${version}.\n` +
        `Přidej nahoru sekci "## [${version}] — <datum>" a napiš do ní, co je nového.\n` +
        `Sekce, které tam jsou: ${listVersions(markdown).join(', ') || '(žádné)'}`,
    )
    process.exit(1)
  }

  process.stdout.write(notes)
}
