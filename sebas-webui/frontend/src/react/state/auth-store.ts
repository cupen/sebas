/**
 * Auth/shell store (2.6). Owns the auth probe and login/setup/logout
 * transitions; a generation guard makes a slow `/api/auth/me` response
 * harmless after a newer login/logout.
 */

import type { AuthInfo } from '../../api/client.js'
import { createStore, Generation, type Store } from './store.js'

export type AuthStatus = 'unknown' | 'anonymous' | 'setup-required' | 'authenticated'

export interface AuthState {
  status: AuthStatus
  auth: AuthInfo | null
  loading: boolean
  error: string | null
}

export interface AuthApi {
  authMe: () => Promise<AuthInfo>
  authLogin: (username: string, password: string) => Promise<unknown>
  authSetup: (username: string, password: string) => Promise<unknown>
  authLogout: () => Promise<unknown>
}

function statusOf(auth: AuthInfo): AuthStatus {
  if (!auth.enabled || auth.authenticated) return 'authenticated'
  return auth.needs_setup ? 'setup-required' : 'anonymous'
}

export class AuthStore {
  readonly store: Store<AuthState>
  private readonly generation = new Generation()
  private readonly api: AuthApi

  constructor(api: AuthApi, initial: AuthState = {
    status: 'unknown',
    auth: null,
    loading: false,
    error: null,
  }) {
    this.api = api
    this.store = createStore(initial)
  }

  getState(): AuthState {
    return this.store.getState()
  }

  subscribe(listener: (state: AuthState) => void): () => void {
    return this.store.subscribe(listener)
  }

  /** Probe `/api/auth/me`; later probes win over earlier ones. */
  async refresh(): Promise<AuthState> {
    const token = this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: true, error: null }))
    try {
      const auth = await this.api.authMe()
      if (!this.generation.isCurrent(token)) return this.getState()
      return this.commit({ status: statusOf(auth), auth, loading: false, error: null })
    } catch (error) {
      if (!this.generation.isCurrent(token)) return this.getState()
      return this.commit({
        status: 'unknown',
        auth: null,
        loading: false,
        error: messageOf(error),
      })
    }
  }

  async login(username: string, password: string): Promise<AuthState> {
    return this.authenticate(() => this.api.authLogin(username, password))
  }

  async setup(username: string, password: string): Promise<AuthState> {
    return this.authenticate(() => this.api.authSetup(username, password))
  }

  async logout(): Promise<AuthState> {
    const token = this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: true, error: null }))
    try {
      await this.api.authLogout()
    } catch {
      /* a failed logout still clears the local session view */
    }
    if (!this.generation.isCurrent(token)) return this.getState()
    return this.commit({ status: 'anonymous', auth: null, loading: false, error: null })
  }

  private async authenticate(run: () => Promise<unknown>): Promise<AuthState> {
    const token = this.generation.next()
    this.store.setState((prev) => ({ ...prev, loading: true, error: null }))
    try {
      await run()
      const auth = await this.api.authMe()
      if (!this.generation.isCurrent(token)) return this.getState()
      return this.commit({ status: statusOf(auth), auth, loading: false, error: null })
    } catch (error) {
      if (!this.generation.isCurrent(token)) return this.getState()
      // Commit the store error first, then rethrow the original ApiError so
      // the view can render its status-specific message inline (401/409/429).
      this.commit((prev) => ({ ...prev, loading: false, error: messageOf(error) }))
      throw error
    }
  }

  private commit(next: AuthState | ((prev: AuthState) => AuthState)): AuthState {
    this.store.setState(next)
    return this.getState()
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
