import { describe, expect, it, vi } from 'vitest'
import type { AuthInfo } from '../../api/client.js'
import { AuthStore, type AuthApi } from './auth-store.js'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function auth(patch: Partial<AuthInfo> = {}): AuthInfo {
  return { enabled: true, authenticated: false, username: null, needs_setup: false, ...patch }
}

function api(overrides: Partial<AuthApi> = {}): AuthApi {
  return {
    authMe: vi.fn(async () => auth({ authenticated: true, username: 'op' })),
    authLogin: vi.fn(async () => ({})),
    authSetup: vi.fn(async () => ({})),
    authLogout: vi.fn(async () => ({})),
    ...overrides,
  }
}

describe('AuthStore', () => {
  it('derives setup-required / anonymous / authenticated from /auth/me', async () => {
    await expect(
      new AuthStore(api({ authMe: async () => auth({ needs_setup: true }) })).refresh(),
    ).resolves.toMatchObject({ status: 'setup-required' })

    await expect(
      new AuthStore(
        api({ authMe: async () => auth({ authenticated: false, needs_setup: false }) }),
      ).refresh(),
    ).resolves.toMatchObject({
      status: 'anonymous',
    })

    await expect(
      new AuthStore(api({ authMe: async () => auth({ enabled: false }) })).refresh(),
    ).resolves.toMatchObject({ status: 'authenticated' })
  })

  it('probes, toggles loading, and re-probes after login', async () => {
    const probe = deferred<AuthInfo>()
    const login = vi.fn(async () => ({}))
    const store = new AuthStore(
      api({
        authMe: vi
          .fn()
          .mockImplementationOnce(() => probe.promise)
          .mockResolvedValueOnce(auth({ authenticated: true, username: 'op' })),
        authLogin: login,
      }),
    )
    const pending = store.refresh()
    expect(store.getState().loading).toBe(true)
    probe.resolve(auth({ needs_setup: true }))
    await pending
    expect(store.getState()).toMatchObject({ status: 'setup-required', loading: false })

    await store.login('op', 'secret')
    expect(login).toHaveBeenCalledWith('op', 'secret')
    expect(store.getState()).toMatchObject({
      status: 'authenticated',
      loading: false,
      auth: { username: 'op' },
    })
  })

  it('keeps the previous status, commits the error and rethrows when login fails', async () => {
    const store = new AuthStore(
      api({
        authMe: async () => auth({ needs_setup: false }),
        authLogin: async () => {
          throw new Error('bad password')
        },
      }),
    )
    await store.refresh()
    expect(store.getState().status).toBe('anonymous')
    await expect(store.login('op', 'wrong')).rejects.toThrow('bad password')
    expect(store.getState()).toMatchObject({
      status: 'anonymous',
      error: 'bad password',
      loading: false,
    })
  })

  it('ignores a stale probe after a newer refresh', async () => {
    const first = deferred<AuthInfo>()
    const second = deferred<AuthInfo>()
    const me = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise)
    const store = new AuthStore(api({ authMe: me }))

    const a = store.refresh()
    const b = store.refresh()
    second.resolve(auth({ authenticated: true, username: 'new' }))
    await b
    first.resolve(auth({ needs_setup: true }))
    await a
    expect(store.getState()).toMatchObject({ status: 'authenticated', auth: { username: 'new' } })
  })

  it('clears the local session view even when logout I/O fails', async () => {
    const store = new AuthStore(
      api({
        authLogout: async () => {
          throw new Error('network down')
        },
      }),
    )
    await store.refresh()
    const state = await store.logout()
    expect(state).toEqual({ status: 'anonymous', auth: null, loading: false, error: null })
  })
})
