// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { base64Size, issueBody, issueTitle, neutralise, safeFileName, validateFeedback, type Feedback } from './feedback'

const valid = {
  kind: 'bug',
  message: 'Widget po restartu zmizí.\nStane se to vždycky.',
  context: { version: '0.4.0', os: 'macos (aarch64)', location: 'Okno aplikace › Nastavení' },
  author: { name: 'Lumír', email: 'lumir@example.com' },
  attachment: null,
}

function ok(input: unknown): Feedback {
  const result = validateFeedback(input)
  if (!result.ok) throw new Error(result.error)
  return result.feedback
}

describe('kontrola tvaru', () => {
  it('přijme platnou zprávu a ořízne okraje', () => {
    const feedback = ok({ ...valid, message: '  Nejde to.  ' })
    expect(feedback.message).toBe('Nejde to.')
    expect(feedback.author).toEqual({ name: 'Lumír', email: 'lumir@example.com' })
  })

  it('odmítne neznámý druh, prázdnou i příliš dlouhou zprávu', () => {
    expect(validateFeedback({ ...valid, kind: 'spam' })).toEqual({ ok: false, error: 'Neznámý druh zpětné vazby.' })
    expect(validateFeedback({ ...valid, message: '' })).toEqual({ ok: false, error: 'Zpráva je prázdná.' })
    expect(validateFeedback({ ...valid, message: 'x'.repeat(10_001) })).toEqual({ ok: false, error: 'Zpráva je příliš dlouhá.' })
    expect(validateFeedback(null).ok).toBe(false)
  })

  it('prázdný autor je anonym, špatný e-mail se nepřijme', () => {
    expect(ok({ ...valid, author: { name: ' ', email: '' } }).author).toBeNull()
    expect(validateFeedback({ ...valid, author: { name: '', email: 'ne-email' } })).toEqual({ ok: false, error: 'E-mail nemá platný tvar.' })
  })

  it('příloha musí mít povolený typ s odpovídající příponou a nejvýš 2 MB', () => {
    const png = { name: 'a.png', type: 'image/png', data: 'AAAA' }
    expect(ok({ ...valid, attachment: png }).attachment).toEqual(png)
    expect(validateFeedback({ ...valid, attachment: { ...png, name: 'a.exe' } }).ok).toBe(false)
    expect(validateFeedback({ ...valid, attachment: { ...png, type: 'application/x-msdownload' } }).ok).toBe(false)
    expect(validateFeedback({ ...valid, attachment: { ...png, data: 'neni base64!' } })).toEqual({ ok: false, error: 'Příloha není platně zakódovaná.' })
    // One byte over 2 MB once decoded.
    const big = 'A'.repeat(4 * Math.ceil((2 * 1024 * 1024 + 1) / 3))
    expect(validateFeedback({ ...valid, attachment: { ...png, data: big } })).toEqual({ ok: false, error: 'Příloha je větší než 2 MB.' })
  })
})

describe('pomocníci', () => {
  it('spočítá velikost base64 bez dekódování', () => {
    expect(base64Size('AAAA')).toBe(3)
    expect(base64Size('AAA=')).toBe(2)
    expect(base64Size('AA==')).toBe(1)
    expect(base64Size('AAA')).toBeNull()
  })

  it('udělá z názvu přílohy bezpečnou cestu', () => {
    expect(safeFileName('Snímek obrazovky 2026-10-28.png')).toBe('Snimek-obrazovky-2026-10-28.png')
    expect(safeFileName('..\\..\\tajne/../soubor.txt')).toBe('soubor.txt')
    expect(safeFileName('...')).toBe('priloha')
  })

  it('zneškodní zmínky, ale e-mail nechá být', () => {
    expect(neutralise('ahoj @octocat')).toBe('ahoj @⁠octocat')
    expect(neutralise('pis na lumir@example.com')).toBe('pis na lumir@example.com')
  })
})

describe('issue', () => {
  it('název má druh a první řádek, dlouhý se zkrátí', () => {
    expect(issueTitle(ok(valid))).toBe('[Chyba] Widget po restartu zmizí.')
    const long = issueTitle(ok({ ...valid, message: 'a'.repeat(120) }))
    expect(long.length).toBeLessThanOrEqual(90)
    expect(long.endsWith('…')).toBe(true)
  })

  it('tělo cituje zprávu a pod ní dá kontext', () => {
    const body = issueBody(ok(valid), { id: 'abcd1234', receivedAt: '2026-10-28T10:00:00.000Z', attachmentUrl: null })
    expect(body).toContain('> Widget po restartu zmizí.\n> Stane se to vždycky.')
    expect(body).toContain('| Systém | macos (aarch64) |')
    expect(body).toContain('| Od | Lumír <lumir@example.com> |')
    expect(body).toContain('| ID | abcd1234 |')
    expect(body).toContain('Odesláno z aplikace Skylimit.')
  })

  it('anonym a soubor, který není obrázek', () => {
    const feedback = ok({ ...valid, author: null, attachment: { name: 'log.txt', type: 'text/plain', data: 'AAAA' } })
    const body = issueBody(feedback, { id: 'x', receivedAt: 't', attachmentUrl: 'https://example.com/log.txt?raw=true' })
    expect(body).toContain('| Od | anonymně |')
    expect(body).toContain('Příloha: [log.txt](https://example.com/log.txt?raw=true)')
  })
})
