import { useState, type FormEvent } from 'react'
import { Alert, Button, Card, Input, Typography } from 'antd'
import { ApiError, api, errorText } from '../../api/client.js'
import styles from './auth.module.css'

export function validateLogin(username: string, password: string): string | null {
  if (!username.trim()) return '请输入用户名'
  if (!password) return '请输入密码'
  return null
}

export function loginErrorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 429) return '尝试次数过多，请稍后再试'
    if (error.status === 401) return '用户名或密码错误'
    return `登录失败（HTTP ${error.status}）：${error.message}`
  }
  return '网络连接失败，请检查服务是否可用'
}

export interface LoginViewProps {
  login?: (username: string, password: string) => Promise<unknown>
  onSuccess?: () => void | Promise<void>
}

/** Login gate for auth-enabled deployments (3.2). */
export function LoginView({ login = api.authLogin, onSuccess }: LoginViewProps) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    if (busy) return
    const invalid = validateLogin(username, password)
    if (invalid !== null) {
      setError(invalid)
      return
    }
    setBusy(true)
    setError(null)
    try {
      await login(username.trim(), password)
      await onSuccess?.()
    } catch (failure) {
      setError(loginErrorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={styles.screen}>
      <Card className={styles.card} variant="borderless">
        <Brand />
        <Typography.Title level={4} className={styles.title}>
          登录以继续
        </Typography.Title>
        <form className={styles.form} noValidate onSubmit={(event) => void submit(event)}>
          <label className={styles.label}>
            用户名
            <Input
              name="username"
              autoComplete="username"
              autoFocus
              value={username}
              onChange={(event) => setUsername(event.target.value)}
            />
          </label>
          <label className={styles.label}>
            密码
            <Input.Password
              name="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
          </label>
          {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
          <Button type="primary" htmlType="submit" loading={busy} block>
            {busy ? '登录中…' : '登录'}
          </Button>
        </form>
      </Card>
    </div>
  )
}

export function Brand() {
  return (
    <div className={styles.brand}>
      <span className={styles.mark} aria-hidden="true">
        &gt;
      </span>
      <span>
        <span className={styles.name}>sebas</span>
        <span className={styles.tagline}>agent console</span>
      </span>
    </div>
  )
}

/** Error text used when a non-API error escapes (kept for parity). */
export function genericError(error: unknown): string {
  return errorText(error)
}
