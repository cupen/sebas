/**
 * Wire shapes of the api client's mutation endpoints, asserted against a
 * stubbed fetch so the exact request body is pinned (the review-card loop
 * depends on the backend's nested `{"decision": <PermissionDecision>}`
 * shape — see session_backend.rs `PermissionDecision`).
 */

import { afterEach, describe, expect, it, vi } from 'vitest'
import { api, ApiError, parseBackendHint } from './client.js'

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

  it('createSession forwards prompt, project_dir and the backend hint', async () => {
    vi.stubGlobal('fetch', fetchMock)
    fetchMock.mockResolvedValue(okResponse({ key: 'oc_k' }))

    await api.createSession('do things', '/tmp/proj', 'native')

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/sessions')
    expect(JSON.parse(String(init.body))).toEqual({
      prompt: 'do things',
      project_dir: '/tmp/proj',
      backend: 'native',
      model: null,
    })

    await api.createSession('inbox task', null)
    const [, init2] = fetchMock.mock.calls[1] as [string, RequestInit]
    expect(JSON.parse(String(init2.body))).toEqual({
      prompt: 'inbox task',
      project_dir: null,
      backend: null,
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
