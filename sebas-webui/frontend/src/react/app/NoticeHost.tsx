import { useEffect, useState } from 'react'
import { Alert, Button, Typography } from 'antd'
import {
  dismiss,
  subscribeNotices,
  type FatalNotice,
  type NoticeItem,
  type NoticeState,
} from '../../notify.js'
import styles from './notice.module.css'

export interface NoticeHostProps {
  onRetry: () => void
}

/** One transient toast; owns its auto-dismiss timer (0 = sticky). */
function NoticeToast({ item }: { item: NoticeItem }) {
  useEffect(() => {
    if (item.duration <= 0) return
    const timer = window.setTimeout(() => dismiss(item.id), item.duration)
    return () => window.clearTimeout(timer)
  }, [item.id, item.duration])

  return (
    <Alert
      type={item.level === 'error' ? 'error' : item.level === 'warn' ? 'warning' : 'info'}
      showIcon
      closable
      title={item.message}
      onClose={() => dismiss(item.id)}
      className={styles.toast}
    />
  )
}

function fatalText(fatal: FatalNotice): string {
  const cause = fatal.cause ? `：${fatal.cause}` : ''
  switch (fatal.kind) {
    case 'startup_failed':
      return `核心启动失败${cause}`
    case 'auth_rejected':
      return `核心拒绝了当前凭据${cause}`
    case 'disconnected':
      return `核心连接已断开${cause}`
    default:
      return `核心不可达${cause}`
  }
}

/** Global toast stack, disconnect banner and fatal lock (3.1). */
export function NoticeHost({ onRetry }: NoticeHostProps) {
  const [state, setState] = useState<NoticeState>({
    items: [],
    wsDown: false,
    fatal: null,
  })

  useEffect(() => subscribeNotices(setState), [])

  return (
    <>
      {state.wsDown && state.fatal === null ? (
        <div className={styles.banner} role="status">
          <Alert
            type="warning"
            showIcon
            banner
            title="服务器连接已断开，正在重连…"
          />
        </div>
      ) : null}

      <div className={styles.stack} aria-live="polite" aria-atomic="false">
        {state.items.map((item) => (
          <NoticeToast key={item.id} item={item} />
        ))}
      </div>

      {state.fatal !== null ? (
        <div className={styles.lock} role="alertdialog" aria-modal="true" aria-label="核心不可达">
          <div className={styles.lockCard}>
            <Typography.Title level={4} className={styles.lockTitle}>
              {fatalText(state.fatal)}
            </Typography.Title>
            <Typography.Paragraph type="secondary" className={styles.lockText}>
              工作台已锁定，避免在核心不可达时继续发出写操作。恢复连接后自动解除。
            </Typography.Paragraph>
            <Button type="primary" onClick={onRetry}>
              重试连接
            </Button>
          </div>
        </div>
      ) : null}
    </>
  )
}
