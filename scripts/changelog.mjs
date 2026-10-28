#!/usr/bin/env node
/**
 * Vytáhnout z CHANGELOG.md sekci k jedné verzi (převzato z Pilcrow).
 *
 * Používá to vydávací workflow: text, který tady vznikne, se stane popisem
 * vydání na GitHubu.
 *
 * Když sekce chybí nebo je prázdná, skončí to chybou. To je záměr: vydání bez
 * poznámek se nemá sestavit. Napsat, co je nového, je součást vydání.
 *
 * Je to obyčejný JavaScript, ne TypeScript v `src/`, protože to musí umět
 * spustit Node přímo v CI, kde se nic nepřekládá.
 */

import { readFileSync } from 'node:fs'

/** Nadpis sekce: `## 1.2.3` nebo `## 1.2.3 — cokoliv` (datum, „nevydáno“). */
function headingFor(version) {
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  return new RegExp(`^##\\s+v?${escaped}\\s*(?:[—–-].*)?$`)
}

/**
 * Text sekce k dané verzi, nebo `null`, když tam není.
 *
 * Bere všechno až po další nadpis druhé úrovně. Samotný nadpis se nevrací --
 * v okně aktualizace už je číslo verze vidět jinde a v popisu vydání taky.
 */
export function extractRelease(markdown, version) {
  const lines = markdown.split(/\r?\n/)
  const heading = headingFor(version)

  const start = lines.findIndex((line) => heading.test(line))
  if (start === -1) return null

  const rest = lines.slice(start + 1)
  const end = rest.findIndex((line) => /^##\s/.test(line))
  const body = (end === -1 ? rest : rest.slice(0, end)).join('\n').trim()

  return body === '' ? null : body
}

/** Verze, ke kterým už sekce existuje. Pro hlášku, když se nenajde ta hledaná. */
export function listVersions(markdown) {
  return markdown
    .split(/\r?\n/)
    .map((line) => /^##\s+v?(\d+\.\d+\.\d+)/.exec(line)?.[1])
    .filter((value) => value !== undefined)
}

// --- spuštění z příkazové řádky --------------------------------------------

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, '/'))

if (isMain) {
  const version = process.argv[2]
  if (!version) {
    console.error('Použití: node scripts/changelog.mjs <verze>   (například 0.6.2)')
    process.exit(2)
  }

  const markdown = readFileSync(new URL('../CHANGELOG.md', import.meta.url), 'utf8')
  const notes = extractRelease(markdown, version)

  if (notes === null) {
    console.error(
      `V CHANGELOG.md není co říct k verzi ${version}.\n` +
        `Přidej nahoru sekci "## ${version} — <datum>" a napiš do ní, co je nového.\n` +
        `Sekce, které tam jsou: ${listVersions(markdown).join(', ') || '(žádné)'}`,
    )
    process.exit(1)
  }

  process.stdout.write(notes)
}
