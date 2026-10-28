/**
 * Worker, který přijme zpětnou vazbu ze Skylimitu a založí z ní issue.
 *
 *   POST /v1/feedback   -- JSON podle `Feedback` v `feedback.ts`
 *   GET  /              -- „žiju“, pro kontrolu nasazení
 *
 * Kam se doručuje, určuje `GITHUB_REPO` (soukromé repo) a čím, `GITHUB_TOKEN`
 * (tajemství Workeru, nikdy ne v aplikaci). Příloha se uloží do téhož repa
 * a issue na ni odkáže; uvidí ji jen ten, kdo do repa smí.
 *
 * Proti zneužití: na jednu IP pár odeslání za minutu, pevné limity velikosti,
 * jen JSON a jen s hlavičkou, kterou posílá aplikace. Nic z toho není
 * tajemství; je to síto na roboty, ne zámek. IP adresa se nikam neukládá.
 */

import { issueBody, issueTitle, KIND_ISSUE_LABEL, LIMITS, safeFileName, validateFeedback, type Feedback } from './feedback'

/** Rate Limiting binding Workerů. Volitelný: v testech a bez něj se neomezuje. */
interface RateLimiter {
  limit(options: { key: string }): Promise<{ success: boolean }>
}

export interface Env {
  GITHUB_TOKEN?: string
  /** `vlastník/název`. */
  GITHUB_REPO?: string
  LIMITER?: RateLimiter
}

export interface Deps {
  fetch: typeof fetch
  now: () => Date
  id: () => string
}

const defaultDeps: Deps = {
  fetch: (input, init) => fetch(input, init),
  now: () => new Date(),
  id: () => crypto.randomUUID().slice(0, 8),
}

function json(status: number, body: Record<string, unknown>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8' },
  })
}

function fail(status: number, error: string): Response {
  return json(status, { ok: false, error })
}

/** Volání GitHub API s tokenem Workeru. */
async function github(deps: Deps, env: Env, method: string, path: string, body: unknown): Promise<Record<string, unknown>> {
  const response = await deps.fetch(`https://api.github.com/repos/${env.GITHUB_REPO}${path}`, {
    method,
    headers: {
      authorization: `Bearer ${env.GITHUB_TOKEN}`,
      accept: 'application/vnd.github+json',
      'x-github-api-version': '2022-11-28',
      'user-agent': 'skylimit-feedback',
      'content-type': 'application/json',
    },
    body: JSON.stringify(body),
  })
  const text = await response.text()
  if (!response.ok) {
    // Do logu (`wrangler tail`), ne odesílateli: odpověď GitHubu může
    // prozradit, jak je doručování nastavené.
    console.error(`GitHub ${method} ${path}: ${response.status} ${text.slice(0, 500)}`)
    throw new Error(`GitHub odpověděl ${response.status}`)
  }
  return text ? (JSON.parse(text) as Record<string, unknown>) : {}
}

/** Uložit přílohu do repozitáře. Vrací adresu, na kterou může issue odkázat. */
async function storeAttachment(deps: Deps, env: Env, feedback: Feedback, id: string, now: Date): Promise<string | null> {
  const attachment = feedback.attachment
  if (!attachment) return null
  const month = `${now.getUTCFullYear()}/${String(now.getUTCMonth() + 1).padStart(2, '0')}`
  const path = `attachments/${month}/${id}-${safeFileName(attachment.name)}`
  const created = await github(deps, env, 'PUT', `/contents/${path}`, {
    message: `Příloha ke zpětné vazbě ${id}`,
    content: attachment.data,
  })
  const content = created.content as Record<string, unknown> | undefined
  const htmlUrl = typeof content?.html_url === 'string' ? content.html_url : null
  // `?raw=true` místo `download_url`: ten má u soukromého repa dočasný token
  // a obrázek by v issue za pár minut zmizel.
  return htmlUrl ? `${htmlUrl}?raw=true` : null
}

export async function handle(request: Request, env: Env, deps: Deps = defaultDeps): Promise<Response> {
  const url = new URL(request.url)

  if (request.method === 'GET' && url.pathname === '/') {
    return new Response('skylimit-feedback\n', { headers: { 'content-type': 'text/plain; charset=utf-8' } })
  }
  if (url.pathname !== '/v1/feedback') return fail(404, 'Tahle adresa neexistuje.')
  if (request.method !== 'POST') return fail(405, 'Sem se jen posílá.')

  if (!env.GITHUB_TOKEN || !env.GITHUB_REPO) {
    console.error('Chybí GITHUB_TOKEN nebo GITHUB_REPO.')
    return fail(503, 'Příjem zpětné vazby teď není nastavený.')
  }
  if (!request.headers.get('x-skylimit-version')) return fail(400, 'Sem posílá jen aplikace Skylimit.')
  if (!(request.headers.get('content-type') ?? '').startsWith('application/json')) {
    return fail(415, 'Zpětná vazba se posílá jako JSON.')
  }
  const declared = Number(request.headers.get('content-length') ?? '0')
  if (declared > LIMITS.body) return fail(413, 'Zpráva je moc velká.')

  if (env.LIMITER) {
    const key = request.headers.get('cf-connecting-ip') ?? 'neznama'
    const { success } = await env.LIMITER.limit({ key })
    if (!success) return fail(429, 'Zpráv přišlo moc najednou. Zkus to za minutu.')
  }

  const raw = await request.text()
  if (raw.length > LIMITS.body) return fail(413, 'Zpráva je moc velká.')
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return fail(400, 'Požadavek není platný JSON.')
  }
  const result = validateFeedback(parsed)
  if (!result.ok) return fail(400, result.error)

  const feedback = result.feedback
  const now = deps.now()
  const id = deps.id()
  try {
    const attachmentUrl = await storeAttachment(deps, env, feedback, id, now)
    const issue = await github(deps, env, 'POST', '/issues', {
      title: issueTitle(feedback),
      body: issueBody(feedback, { id, receivedAt: now.toISOString(), attachmentUrl }),
      labels: [KIND_ISSUE_LABEL[feedback.kind]],
    })
    return json(200, { ok: true, id, number: typeof issue.number === 'number' ? issue.number : null })
  } catch {
    return fail(502, 'Zpětnou vazbu se nepodařilo uložit. Zkus to prosím později.')
  }
}

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env)
  },
}
