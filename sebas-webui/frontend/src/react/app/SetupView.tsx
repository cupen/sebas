import { useState, type FormEvent } from 'react'
import { Alert, Button, Card, Input, Typography } from 'antd'
import { ApiError, api } from '../../api/client.js'
import { Brand } from './LoginView.js'
import styles from './auth.module.css'

/** Same minimum as the server and the Lit setup view. */
export const MIN_PASSWORD_LENGTH = 8

export function validateSetup(
  username: string,
  password: string,
  confirm: string,
): string | null {
  if (!username.trim()) return '请输入用户名'
  if (password.length < MIN_PASSWORD_LENGTH) return `密码至少需要 ${MIN_PASSWORD_LENGTH} 个字符`
  if (confirm !== password) return '两次输入的密码不一致'
  return null
}

export function setupErrorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return error.status === 409 ? `初始化被拒绝：${error.message}` : error.message
  }
  return '网络连接失败，请检查服务是否可用'
}

export interface SetupViewProps {
  setup?: (username: string, password: string) => Promise<unknown>
  onSuccess?: () => void | Promise<void>
}

/** Zero-user first-run root setup (3.2). */
export function SetupView({ setup = api.authSetup, onSuccess }: SetupViewProps) {
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(event: FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault()
    if (busy) return
    const invalid = validateSetup(username, password, confirm)
    if (invalid !== null) {
      setError(invalid)
      return
    }
    setBusy(true)
    setError(null)
    try {
      await setup(username.trim(), password)
      await onSuccess?.()
    } catch (failure) {
      setError(setupErrorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={styles.screen}>
      <Card className={styles.card} variant="borderless">
        <Brand />
        <Typography.Title level={4} className={styles.title}>
          创建管理员账户
        </Typography.Title>
        <Typography.Paragraph type="secondary" className={styles.hint}>
          这是此实例的第一个账户（root），之后可在设置内管理其他用户。
        </Typography.Paragraph>
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
            密码（至少 {MIN_PASSWORD_LENGTH} 个字符）
            <Input.Password
              name="password"
              autoComplete="new-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
            />
          </label>
          <label className={styles.label}>
            确认密码
            <Input.Password
              name="confirm"
              autoComplete="new-password"
              value={confirm}
              onChange={(event) => setConfirm(event.target.value)}
            />
          </label>
          {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
          <Button type="primary" htmlType="submit" loading={busy} block>
            {busy ? '创建中…' : '创建并进入'}
          </Button>
        </form>
      </Card>
    </div>
  )
}
