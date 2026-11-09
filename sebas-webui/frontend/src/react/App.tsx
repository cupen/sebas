import { useEffect, useMemo } from 'react'
import { Alert, Button, Spin } from 'antd'
import { api, setUnauthorizedHandler } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { AuthStore } from './state/auth-store.js'
import { SessionStore } from './state/session-store.js'
import { useStore } from './state/use-store.js'
import { LoginView } from './app/LoginView.js'
import { SetupView } from './app/SetupView.js'
import { Shell } from './app/Shell.js'
import styles from './app/auth.module.css'

/**
 * Application root: routes, auth gate, theme provider and notice host.
 *
 * Since the 5.3 cutover this is the only production entry — `index.html`
 * loads `src/react/main.tsx`, which mounts this tree against the same
 * HTTP/WS surface the Rust binary serves.
 */
export function App() {
  const auth = useMemo(
    () =>
      new AuthStore({
        authMe: api.authMe,
        authLogin: api.authLogin,
        authSetup: api.authSetup,
        authLogout: api.authLogout,
      }),
    [],
  )
  const sessions = useMemo(
    () =>
      new SessionStore({
        summary: api.summary,
        sessions: api.sessions,
        projects: () => api.projects.list(),
      }),
    [],
  )

  const status = useStore(auth.store, (state) => state.status)
  const loading = useStore(auth.store, (state) => state.loading)
  const error = useStore(auth.store, (state) => state.error)
  const info = useStore(auth.store, (state) => state.auth)

  useEffect(() => {
    void auth.refresh()
  }, [auth])

  useEffect(() => {
    setUnauthorizedHandler(() => {
      sharedWs.setAuthGated(true)
      void auth.refresh()
    })
    return () => setUnauthorizedHandler(null)
  }, [auth])

  useEffect(() => {
    if (status === 'authenticated') {
      sharedWs.setAuthGated(false)
      void sessions.refresh()
      return
    }
    if (status === 'anonymous' || status === 'setup-required') {
      sharedWs.setAuthGated(true)
    }
  }, [status, sessions])

  if (status === 'unknown') {
    return (
      <div className={styles.screen}>
        {error !== null && !loading ? (
          <Alert
            type="error"
            showIcon
            title={`无法连接服务：${error}`}
            action={<Button size="small" onClick={() => void auth.refresh()}>重试</Button>}
            role="alert"
          />
        ) : (
          <Spin description="正在检查登录状态…" data-testid="auth-boot" />
        )}
      </div>
    )
  }

  if (status === 'setup-required') {
    return (
      <SetupView
        setup={(username, password) => auth.setup(username, password)}
        onSuccess={async () => { await auth.refresh() }}
      />
    )
  }

  if (status === 'anonymous') {
    return (
      <LoginView
        login={(username, password) => auth.login(username, password)}
        onSuccess={async () => { await auth.refresh() }}
      />
    )
  }

  return (
    <Shell
      auth={auth}
      sessions={sessions}
      role={info?.role ?? null}
      username={info?.username ?? null}
    />
  )
}
