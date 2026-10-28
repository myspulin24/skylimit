// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { handle, type Deps, type Env } from './index'

const env: Env = { GITHUB_TOKEN: 'tajne', GITHUB_REPO: 'o/feedback' }

const body = {
  kind: 'change',
  message: 'Chci tmavší widget.',
  context: { version: '0.4.0', os: 'windows (x86_64)', location: 'Okno aplikace › Měsíc' },
  author: null,
  attachment: null,
}

function post(payload: unknown, headers: Record<string, string> = {}): Request {
  return new Request('https://feedback.example/v1/feedback', {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-skylimit-version': '0.4.0', ...headers },
    body: typeof payload === 'string' ? payload : JSON.stringify(payload),
  })
}

/** Podvržený GitHub: zapamatuje si volání a odpoví, jak se mu řekne. */
function fakeGithub(respond: (url: string) => Response = () => Response.json({ number: 7 }, { status: 201 })) {
  const calls: Array<{ url: string; method: string; body: Record<string, unknown>; auth: string | null }> = []
  const deps: Deps = {
    fetch: async (input, init = {}) => {
      const url = String(input)
      calls.push({
        url,
        method: init.method ?? 'GET',
        body: JSON.parse(String(init.body ?? '{}')) as Record<string, unknown>,
        auth: new Headers(init.headers).get('authorization'),
      })
      return respond(url)
    },
    now: () => new Date('2026-10-28T10:00:00Z'),
    id: () => 'abcd1234',
  }
  return { calls, deps }
}

describe('obsluha', () => {
  it('založí issue v nastaveném repu se štítkem podle druhu', async () => {
    const { calls, deps } = fakeGithub()
    const response = await handle(post(body), env, deps)

    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ ok: true, id: 'abcd1234', number: 7 })
    expect(calls).toHaveLength(1)
    expect(calls[0]?.url).toBe('https://api.github.com/repos/o/feedback/issues')
    expect(calls[0]?.auth).toBe('Bearer tajne')
    expect(calls[0]?.body.title).toBe('[Změna] Chci tmavší widget.')
    expect(calls[0]?.body.labels).toEqual(['změna'])
  })

  it('přílohu napřed uloží do repa a issue na ni odkáže', async () => {
    const { calls, deps } = fakeGithub((url) =>
      url.includes('/contents/')
        ? Response.json({ content: { html_url: 'https://github.com/o/feedback/blob/main/attachments/2026/10/abcd1234-snimek.png' } }, { status: 201 })
        : Response.json({ number: 8 }, { status: 201 }),
    )
    const withFile = { ...body, attachment: { name: 'snímek.png', type: 'image/png', data: 'iVBORw0KGgo=' } }
    const response = await handle(post(withFile), env, deps)

    expect(response.status).toBe(200)
    expect(calls).toHaveLength(2)
    expect(calls[0]?.method).toBe('PUT')
    expect(calls[0]?.url).toBe('https://api.github.com/repos/o/feedback/contents/attachments/2026/10/abcd1234-snimek.png')
    expect(calls[0]?.body.content).toBe('iVBORw0KGgo=')
    expect(String(calls[1]?.body.body)).toContain('![snímek.png](https://github.com/o/feedback/blob/main/attachments/2026/10/abcd1234-snimek.png?raw=true)')
  })

  it('bez hlavičky aplikace, s neplatným JSON nebo tvarem odmítne', async () => {
    const { calls, deps } = fakeGithub()
    const noHeader = new Request('https://feedback.example/v1/feedback', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })
    expect((await handle(noHeader, env, deps)).status).toBe(400)
    expect((await handle(post('{nejson'), env, deps)).status).toBe(400)
    const empty = await handle(post({ ...body, message: '   ' }), env, deps)
    expect(empty.status).toBe(400)
    expect(await empty.json()).toEqual({ ok: false, error: 'Zpráva je prázdná.' })
    expect((await handle(post(body, { 'content-type': 'text/plain' }), env, deps)).status).toBe(415)
    expect(calls).toHaveLength(0)
  })

  it('bez tokenu řekne, že příjem není nastavený', async () => {
    const { calls, deps } = fakeGithub()
    const response = await handle(post(body), { GITHUB_REPO: 'o/feedback' }, deps)
    expect(response.status).toBe(503)
    expect(calls).toHaveLength(0)
  })

  it('omezí počet odeslání z jedné adresy', async () => {
    const { calls, deps } = fakeGithub()
    const limited: Env = { ...env, LIMITER: { limit: async () => ({ success: false }) } }
    const response = await handle(post(body, { 'cf-connecting-ip': '203.0.113.9' }), limited, deps)
    expect(response.status).toBe(429)
    expect(calls).toHaveLength(0)
  })

  it('chybu GitHubu neprozradí odesílateli', async () => {
    const { deps } = fakeGithub(() => new Response('Bad credentials', { status: 401 }))
    const errors: unknown[] = []
    const original = console.error
    console.error = (...args: unknown[]) => errors.push(args)
    try {
      const response = await handle(post(body), env, deps)
      expect(response.status).toBe(502)
      const answer = (await response.json()) as { error: string }
      expect(answer.error).not.toContain('credentials')
      expect(errors).toHaveLength(1)
    } finally {
      console.error = original
    }
  })

  it('odpoví na kontrolu nasazení a jinde vrátí 404 nebo 405', async () => {
    const { deps } = fakeGithub()
    const alive = await handle(new Request('https://feedback.example/'), env, deps)
    expect(await alive.text()).toBe('skylimit-feedback\n')
    expect((await handle(new Request('https://feedback.example/jinde'), env, deps)).status).toBe(404)
    expect((await handle(new Request('https://feedback.example/v1/feedback'), env, deps)).status).toBe(405)
  })
})
