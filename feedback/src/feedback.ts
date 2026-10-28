/**
 * Zpětná vazba ze Skylimitu: co se smí přijmout a jak z toho vznikne issue.
 *
 * Čistá část Workeru: žádná síť, žádné tajemství. Tady se rozhoduje, co je
 * platná zpráva a jak bude vypadat v issue; `index.ts` ji jen doručí.
 *
 * Endpoint je veřejný. Aplikace žádné tajemství nenese, protože co je
 * v instalačce, si kdokoli vytáhne. Ochranou tedy není heslo, ale to, že se
 * přijme jen přesně tenhle tvar s pevnými limity a že všechno skončí
 * v soukromém repozitáři, kde případný spam nikdo cizí neuvidí.
 */

export type FeedbackKind = 'bug' | 'change' | 'improvement'

export interface FeedbackAttachment {
  name: string
  type: string
  /** Obsah v base64. */
  data: string
}

export interface Feedback {
  kind: FeedbackKind
  message: string
  context: {
    version: string
    os: string
    /** Kde v aplikaci uživatel byl (záložka). */
    location: string
  }
  /** `null` = anonymně. */
  author: { name: string; email: string } | null
  attachment: FeedbackAttachment | null
}

/** Limity. Stejné hlídá i aplikace, ale rozhoduje se tady. */
export const LIMITS = {
  /** Celé tělo požadavku; příloha v base64 je o třetinu větší než soubor. */
  body: 3_200_000,
  message: 10_000,
  attachment: 2 * 1024 * 1024,
  short: 200,
} as const

/** Co se smí přiložit: obrázek, PDF, log, CSV, JSON, text. */
const ATTACHMENT_TYPES: Record<string, string[]> = {
  'image/png': ['png'],
  'image/jpeg': ['jpg', 'jpeg'],
  'image/gif': ['gif'],
  'image/webp': ['webp'],
  'application/pdf': ['pdf'],
  'text/plain': ['txt', 'log', 'md'],
  'text/markdown': ['md'],
  'text/csv': ['csv'],
  'application/json': ['json'],
}

export const KIND_LABEL: Record<FeedbackKind, string> = {
  bug: 'Chyba',
  change: 'Změna',
  improvement: 'Vylepšení',
}

/** Štítek v repozitáři se zpětnou vazbou. */
export const KIND_ISSUE_LABEL: Record<FeedbackKind, string> = {
  bug: 'bug',
  change: 'změna',
  improvement: 'vylepšení',
}

export type Validation = { ok: true; feedback: Feedback } | { ok: false; error: string }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Řetězec s horní mezí. Nic jiného než řetězec se nepřijme. */
function text(value: unknown, max: number): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  return trimmed.length <= max ? trimmed : null
}

/** Kolik bajtů je v base64 řetězci, bez dekódování. `null`, když to base64 není. */
export function base64Size(data: string): number | null {
  if (data.length === 0 || data.length % 4 !== 0 || !/^[A-Za-z0-9+/]+={0,2}$/.test(data)) return null
  const padding = data.endsWith('==') ? 2 : data.endsWith('=') ? 1 : 0
  return (data.length / 4) * 3 - padding
}

/** Jméno přílohy, které se dá bezpečně použít jako cesta v repozitáři. */
export function safeFileName(name: string): string {
  const base = name.split(/[\\/]/).pop() ?? ''
  const cleaned = base
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[^A-Za-z0-9._-]+/g, '-')
    .replace(/^[.-]+/, '')
    .replace(/-+/g, '-')
    .slice(-80)
  return cleaned || 'priloha'
}

function validateAttachment(value: unknown): FeedbackAttachment | string | null {
  if (value === null || value === undefined) return null
  if (!isRecord(value)) return 'Příloha nemá platný tvar.'
  const name = text(value.name, LIMITS.short)
  const type = text(value.type, 100)
  const data = typeof value.data === 'string' ? value.data : null
  if (!name || !type || !data) return 'Příloha nemá platný tvar.'
  const extensions = ATTACHMENT_TYPES[type]
  const extension = name.split('.').pop()?.toLowerCase() ?? ''
  if (!extensions || !extensions.includes(extension)) {
    return 'Tenhle typ přílohy se nepřijímá. Jde přiložit obrázek, PDF, log, CSV, JSON nebo text.'
  }
  const size = base64Size(data)
  if (size === null) return 'Příloha není platně zakódovaná.'
  if (size > LIMITS.attachment) return 'Příloha je větší než 2 MB.'
  return { name, type, data }
}

/** Je to zpětná vazba, která se smí doručit? */
export function validateFeedback(input: unknown): Validation {
  if (!isRecord(input)) return { ok: false, error: 'Požadavek nemá platný tvar.' }

  const kind = input.kind
  if (kind !== 'bug' && kind !== 'change' && kind !== 'improvement') {
    return { ok: false, error: 'Neznámý druh zpětné vazby.' }
  }

  const message = text(input.message, LIMITS.message)
  if (message === null) return { ok: false, error: 'Zpráva je příliš dlouhá.' }
  if (message === '') return { ok: false, error: 'Zpráva je prázdná.' }

  const context = isRecord(input.context) ? input.context : {}
  const version = text(context.version ?? '', 40)
  const os = text(context.os ?? '', LIMITS.short)
  const location = text(context.location ?? '', LIMITS.short)
  if (version === null || os === null || location === null) {
    return { ok: false, error: 'Kontext má neplatný tvar.' }
  }

  let author: Feedback['author'] = null
  if (input.author !== null && input.author !== undefined) {
    if (!isRecord(input.author)) return { ok: false, error: 'Autor má neplatný tvar.' }
    const name = text(input.author.name ?? '', 100)
    const email = text(input.author.email ?? '', LIMITS.short)
    if (name === null || email === null) return { ok: false, error: 'Autor má neplatný tvar.' }
    if (email && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) return { ok: false, error: 'E-mail nemá platný tvar.' }
    author = name || email ? { name, email } : null
  }

  const attachment = validateAttachment(input.attachment)
  if (typeof attachment === 'string') return { ok: false, error: attachment }

  return { ok: true, feedback: { kind, message, context: { version, os, location }, author, attachment } }
}

/**
 * Text od uživatele do Markdownu issue, ale bez zmínek.
 *
 * `@nekdo` v issue pošle tomu člověku upozornění; veřejný endpoint by tak
 * šel použít k obtěžování. Neviditelný znak mezi zavináčem a jménem zmínku
 * rozbije a text vypadá pořád stejně.
 */
export function neutralise(value: string): string {
  // Jen `@` na začátku slova; v e-mailu (`jmeno@domena`) zmínka není.
  return value.replace(/(?<![\w.+-])@(?=[A-Za-z0-9-])/g, '@\u2060')
}

/** Jeden řádek do tabulky: bez svislítek a konců řádků, které by ji rozbily. */
function cell(value: string): string {
  return neutralise(value).replace(/\|/g, '\\|').replace(/[\r\n]+/g, ' ')
}

/** Název issue: druh a začátek zprávy. */
export function issueTitle(feedback: Feedback): string {
  const first = (feedback.message.split('\n').find((line) => line.trim()) ?? '').trim()
  const short = first.length > 80 ? `${first.slice(0, 79).trimEnd()}…` : first
  return `[${KIND_LABEL[feedback.kind]}] ${neutralise(short)}`
}

export interface Delivered {
  id: string
  receivedAt: string
  /** Kde v repozitáři leží příloha, když nějaká je. */
  attachmentUrl: string | null
}

/** Tělo issue: zpráva, pod ní všechno, co aplikace přidala sama. */
export function issueBody(feedback: Feedback, delivered: Delivered): string {
  const { context, author, attachment } = feedback
  const rows: Array<[string, string]> = [
    ['Druh', KIND_LABEL[feedback.kind]],
    ['Verze', context.version || '—'],
    ['Systém', context.os || '—'],
    ['Kde', context.location || '—'],
    ['Od', author ? [author.name, author.email && `<${author.email}>`].filter(Boolean).join(' ') : 'anonymně'],
    ['Přijato', delivered.receivedAt],
    ['ID', delivered.id],
  ]

  const quoted = neutralise(feedback.message)
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n')

  const parts = [quoted, '', '| | |', '| --- | --- |', ...rows.map(([key, value]) => `| ${key} | ${cell(value)} |`)]
  if (attachment && delivered.attachmentUrl) {
    const name = cell(attachment.name)
    parts.push(
      '',
      attachment.type.startsWith('image/')
        ? `![${name}](${delivered.attachmentUrl})`
        : `Příloha: [${name}](${delivered.attachmentUrl})`,
    )
  }
  parts.push('', '<sub>Odesláno z aplikace Skylimit.</sub>')
  return parts.join('\n')
}
