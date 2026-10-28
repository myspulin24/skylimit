import { describe, expect, it } from 'vitest'

import { extractRelease, listVersions } from './changelog.mjs'

const CHANGELOG = [
  '# Změny',
  '',
  'Úvodní odstavec, který do žádného vydání nepatří.',
  '',
  '## 0.6.2 — nevydáno',
  '',
  '### Přidáno',
  '',
  '- Něco nového.',
  '',
  '## 0.6.1',
  '',
  '### Opraveno',
  '',
  '- Něco opraveného.',
  '',
  '## 0.6.0 — 2026-09-19',
  '',
  '- Starší.',
  '',
].join('\n')

describe('extractRelease', () => {
  it('vrátí text sekce bez nadpisu', () => {
    expect(extractRelease(CHANGELOG, '0.6.1')).toBe('### Opraveno\n\n- Něco opraveného.')
  })

  it('končí u další verze, nesebere celý zbytek souboru', () => {
    const notes = extractRelease(CHANGELOG, '0.6.2')
    expect(notes).toBe('### Přidáno\n\n- Něco nového.')
    expect(notes).not.toContain('Něco opraveného')
  })

  it('poslední sekce v souboru funguje stejně', () => {
    expect(extractRelease(CHANGELOG, '0.6.0')).toBe('- Starší.')
  })

  it('nadpis snese datum, „nevydáno“ i předponu v', () => {
    expect(extractRelease('## v1.0.0 — 2026-01-01\n\n- Text.', '1.0.0')).toBe('- Text.')
    expect(extractRelease('## 1.0.0 – cokoliv\n\n- Text.', '1.0.0')).toBe('- Text.')
  })

  it('úvod souboru se za poznámky nevydává', () => {
    // Kdyby se hledala neexistující verze a vrátil se text nad první sekcí,
    // dostal by uživatel v okně aktualizace úvod changelogu.
    expect(extractRelease(CHANGELOG, '9.9.9')).toBeNull()
  })

  it('prázdná sekce je stejně špatně jako žádná', () => {
    expect(extractRelease('## 1.0.0\n\n## 0.9.0\n\n- Text.', '1.0.0')).toBeNull()
    expect(extractRelease('## 1.0.0\n   \n\n## 0.9.0', '1.0.0')).toBeNull()
  })

  it('číslo verze se bere doslova, ne jako regulární výraz', () => {
    // Tečka v „1.0.0" nesmí zastupovat libovolný znak a chytit „1x0x0".
    expect(extractRelease('## 1x0x0\n\n- Text.', '1.0.0')).toBeNull()
  })

  it('nesplete si 0.6.1 s 0.6.10', () => {
    const md = '## 0.6.10\n\n- Desítka.\n\n## 0.6.1\n\n- Jednička.'
    expect(extractRelease(md, '0.6.1')).toBe('- Jednička.')
    expect(extractRelease(md, '0.6.10')).toBe('- Desítka.')
  })

  it('zvládne i windowsové konce řádků', () => {
    expect(extractRelease('## 1.0.0\r\n\r\n- Text.\r\n', '1.0.0')).toBe('- Text.')
  })
})

describe('listVersions', () => {
  it('vyjmenuje verze, ke kterým sekce existuje', () => {
    expect(listVersions(CHANGELOG)).toEqual(['0.6.2', '0.6.1', '0.6.0'])
  })
})
