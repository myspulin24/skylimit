import { describe, expect, it } from 'vitest'

import { compareVersions, extractRelease, isValidVersion, listVersions, parseVersion, releaseDate } from './changelog.mjs'

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

describe('formát Keep a Changelog', () => {
  const KAC = [
    '# Změny',
    '',
    '## [Nevydáno]',
    '',
    '## [0.7.0-rc.1] — 2026-11-02',
    '',
    '- Předběžná.',
    '',
    '## [0.6.0] — 2026-10-28',
    '',
    '### Přidáno',
    '',
    '- Updater.',
    '',
    '[Nevydáno]: https://github.com/o/r/compare/v0.6.0...HEAD',
    '[0.6.0]: https://github.com/o/r/compare/v0.5.0...v0.6.0',
  ].join('\n')

  it('najde sekci s verzí v hranatých závorkách a odkazy pod čarou vynechá', () => {
    expect(extractRelease(KAC, '0.6.0')).toBe('### Přidáno\n\n- Updater.')
    expect(extractRelease(KAC, '0.7.0-rc.1')).toBe('- Předběžná.')
  })

  it('[Nevydáno] není verze a datum se čte z nadpisu', () => {
    expect(listVersions(KAC)).toEqual(['0.7.0-rc.1', '0.6.0'])
    expect(releaseDate(KAC, '0.6.0')).toBe('2026-10-28')
    expect(releaseDate(KAC, '9.9.9')).toBeNull()
  })
})

describe('SemVer 2.0.0-rc.2', () => {
  it('platná verze je X.Y.Z bez úvodních nul, s volitelnou předběžnou verzí a sestavením (pravidla 2, 9, 10)', () => {
    for (const ok of ['0.0.0', '0.6.0', '1.0.0', '10.20.30', '1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-0.3.7', '1.0.0-x.7.z.92', '1.0.0+build.1', '1.0.0-rc.1+build.1']) {
      expect(isValidVersion(ok), ok).toBe(true)
    }
    for (const bad of ['1.0', '01.0.0', '1.02.0', '1.0.0-', '1.0.0-al..pha', 'v1.0.0', '1.0.0-alpha_beta', '']) {
      expect(isValidVersion(bad), bad).toBe(false)
    }
    expect(parseVersion('1.2.3-rc.1+b.5')).toEqual({ major: 1, minor: 2, patch: 3, pre: ['rc', '1'], build: ['b', '5'] })
  })

  it('přednost podle pravidla 11 (příklad ze specifikace)', () => {
    const ordered = ['1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-beta.2', '1.0.0-beta.11', '1.0.0-rc.1', '1.0.0']
    for (let i = 0; i < ordered.length - 1; i++) {
      expect(compareVersions(ordered[i], ordered[i + 1]), `${ordered[i]} < ${ordered[i + 1]}`).toBeLessThan(0)
    }
    expect([...ordered].reverse().sort(compareVersions)).toEqual(ordered)
  })

  it('čísla se porovnávají číselně a metadata sestavení se nepočítají', () => {
    expect(compareVersions('0.6.10', '0.6.9')).toBeGreaterThan(0)
    expect(compareVersions('0.10.0', '0.9.9')).toBeGreaterThan(0)
    expect(compareVersions('1.0.0', '0.99.99')).toBeGreaterThan(0)
    expect(compareVersions('1.0.0+a', '1.0.0+b')).toBe(0)
    expect(() => compareVersions('1.0', '1.0.0')).toThrow()
  })
})
