/**
 * Wire shapes of the api client's mutation endpoints, asserted against a
 * stubbed fetch so the exact request body is pinned (the review-card loop
 * depends on the backend's nested `{"decision": <PermissionDecision>}`
 * shape — see session_backend.rs `PermissionDecision`).
 */

import { afterEach, describe, expect, it, vi } from 'vitest'
import { api, ApiError, NetworkError, parseBackendHint, withQuery } from './client.js'

const fetchMock = vi.fn()

function okResponse(body: unknown): Response {
  return { ok: true, json: async () => body } as unknown as Response
}

function errorResponse(status: number, body: unknown): Response {
  return { ok: false, status, json: async () => body } as unknown as Response
}

afterEach(() => {
  vi.unstubAllGlobals()
  fetchMock.mockReset()
})

describe('api wire shapes', () => {
  it('answerPermission posts the internally-tagged decision to the answer route', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ status: 'delivered' }))

    await api.answerPermission('toolu_01ABC', { decision: 'allow_once' })

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/permissions/toolu_01ABC/answer')
    expect(init.method).toBe('POST')
    expect(JSON.parse(String(init.body))).toEqual({ decision: { decision: 'allow_once' } })
  })

  it('answerPermission carries the escalate reason on the wire', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ status: 'delivered' }))

    await api.answerPermission('toolu_2', { decision: 'escalate', reason: 'need network once' })

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(JSON.parse(String(init.body))).toEqual({
      decision: { decision: 'escalate', reason: 'need network once' },
    })
  })

  it('answerPermission rejects with ApiError 404 when nothing is pending', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(
      errorResponse(404, { error: 'no pending permission request with that id' }),
    )

    const err = await api.answerPermission('gone', { decision: 'deny' }).catch((e) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect((err as ApiError).status).toBe(404)
  })

  it('createSession forwards prompt, project_id and the agent id (D2 wire)', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ key: 'oc_k' }))

    await api.createSession({ prompt: 'do things', projectId: 'proj-abc123def456', agent: 'native' })

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/sessions')
    expect(JSON.parse(String(init.body))).toEqual({
      prompt: 'do things',
      project_id: 'proj-abc123def456',
      agent: 'native',
      model: null,
    })

    await api.createSession({ prompt: 'inbox task', agent: 'claudecode' })
    const [, init2] = fetchMock.mock.calls[1] as [string, RequestInit]
    expect(JSON.parse(String(init2.body))).toEqual({
      prompt: 'inbox task',
      project_id: null,
      agent: 'claudecode',
      model: null,
    })
  })

  it('parseBackendHint treats a bare acp as the default agent', () => {
    expect(parseBackendHint('acp')).toEqual({ driver: 'acp' })
  })

  it('parseBackendHint splits acp:<kind> into driver + slug', () => {
    expect(parseBackendHint('acp:gemini')).toEqual({ driver: 'acp', slug: 'gemini' })
  })

  it('parseBackendHint recognises native', () => {
    expect(parseBackendHint('native')).toEqual({ driver: 'native' })
  })
})

describe('withQuery (add-webui-picker-workdir-start)', () => {
  it('URLSearchParams-encodes backslashes, verbatim prefixes and CJK', () => {
    expect(withQuery('/api/fs/browse-dirs', { path: '\\\\?\\D:\\目录' })).toBe(
      '/api/fs/browse-dirs?path=%5C%5C%3F%5CD%3A%5C%E7%9B%AE%E5%BD%95',
    )
  })

  it('encodes + and & that a hand-rolled template risks', () => {
    expect(withQuery('/x', { path: 'a+b&c=d' })).toBe('/x?path=a%2Bb%26c%3Dd')
  })

  it('omits empty/null/undefined values so the server default applies', () => {
    expect(withQuery('/api/fs/browse-dirs', { path: '', root: null, keep: 'v' })).toBe(
      '/api/fs/browse-dirs?keep=v',
    )
  })

  it('fsBrowseDirs omits root when unset (server work-dir default)', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ path: 'X:\\', entries: [] }))
    await api.fsBrowseDirs('')
    const url = fetchMock.mock.calls[0][0] as string
    expect(url).toBe('/api/fs/browse-dirs')
  })

  it('fsBrowseDirs sends an explicit root when given', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ path: 'X:\\w', entries: [] }))
    await api.fsBrowseDirs('sub', 'X:\\w')
    const url = fetchMock.mock.calls[0][0] as string
    expect(url.startsWith('/api/fs/browse-dirs?')).toBe(true)
    expect(url).toContain('root=X%3A%5Cw')
  })
})

describe('network-level failures', () => {
  it('wraps a fetch TypeError into a distinguishable NetworkError', async () => {
    // add-webui-allowed-roots D6：服务进程死亡时 fetch 抛 TypeError（无
    // HTTP 响应），client 统一转成 NetworkError，视图据此区分「后端拒绝」
    // 与「进程没了」。
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockRejectedValue(new TypeError('Failed to fetch'))

    const err = await api.sessions().catch((e) => e)
    expect(err).toBeInstanceOf(NetworkError)
    expect((err as Error).name).toBe('NetworkError')
    expect(err).not.toBeInstanceOf(ApiError)
  })

  it('keeps ApiError for HTTP error responses (no conflation)', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(errorResponse(503, { error: 'backend down' }))

    const err = await api.sessions().catch((e) => e)
    expect(err).toBeInstanceOf(ApiError)
    expect((err as ApiError).status).toBe(503)
  })
})
