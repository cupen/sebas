import { useState } from 'react'
import { Alert, Button, Card, Empty, Input, Modal, Skeleton, Space, Tag, Typography } from 'antd'
import { api, errorText, type SessionRow } from '../../api/client.js'
import type { SessionStore } from '../state/session-store.js'
import { useStore } from '../state/use-store.js'
import { relativeTime, sessionLabel, statusLabel, statusTone } from '../app/labels.js'
import styles from './sessions.module.css'

const TONE_COLOR: Record<string, string> = {
  ok: 'green',
  warn: 'orange',
  err: 'red',
  info: 'blue',
  neutral: 'default',
}

export interface SessionsPageProps {
  sessions: SessionStore
  canCreate: boolean
  onFocus: (key: string) => void
  onRequestCreate: () => void
}

export function SessionsPage({ sessions, canCreate, onFocus, onRequestCreate }: SessionsPageProps) {
  const summary = useStore(sessions.store, (state) => state.summary)
  const list = useStore(sessions.store, (state) => state.list)
  const loading = useStore(sessions.store, (state) => state.loading)
  const error = useStore(sessions.store, (state) => state.error)
  const [actionError, setActionError] = useState<string | null>(null)
  const [rename, setRename] = useState<SessionRow | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [archive, setArchive] = useState<SessionRow | null>(null)
  const [busy, setBusy] = useState(false)

  async function act(run: () => Promise<unknown>): Promise<void> {
    setBusy(true)
    setActionError(null)
    try {
      await run()
      await sessions.refresh()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function commitRename(): Promise<void> {
    if (rename === null) return
    await act(() => api.setSessionLabel(rename.encoded_key, renameValue.trim() || null))
    setRename(null)
  }

  async function commitArchive(): Promise<void> {
    if (archive === null) return
    await act(() => api.archiveSession(archive.encoded_key))
    setArchive(null)
  }

  const rows = list?.recent_sessions ?? []

  return (
    <div className={styles.page} data-testid="sessions-page">
      <header className={styles.head}>
        <div>
          <Typography.Title level={3} className={styles.title}>
            会话总览
          </Typography.Title>
          <Typography.Paragraph type="secondary" className={styles.sub}>
            {canCreate ? '跨项目查看、聚焦与管理会话。' : '只读总览：当前角色不能创建或修改会话。'}
          </Typography.Paragraph>
        </div>
        {canCreate ? (
          <Button type="primary" onClick={onRequestCreate}>
            新建会话
          </Button>
        ) : null}
      </header>

      <div className={styles.chips}>
        <Tag>活跃 {summary?.active_count ?? 0}</Tag>
        <Tag>启动中 {summary?.spawning_count ?? 0}</Tag>
        <Tag>休眠 {summary?.dormant_count ?? 0}</Tag>
        <Tag>总计 {summary?.total_sessions ?? list?.total_sessions ?? 0}</Tag>
      </div>

      {error !== null ? (
        <Alert
          type="error"
          showIcon
          title={`会话加载失败：${error}`}
          action={<Button size="small" onClick={() => void sessions.refresh()}>重试</Button>}
          role="alert"
        />
      ) : null}
      {actionError !== null ? <Alert type="error" showIcon title={actionError} role="alert" /> : null}

      {loading && rows.length === 0 ? <Skeleton active /> : null}
      {!loading && rows.length === 0 && error === null ? (
        <Empty description="还没有会话" />
      ) : null}

      <div className={styles.grid}>
        {rows.map((row) => (
          <Card key={row.encoded_key} size="small" variant="borderless" className={styles.card}>
            <div className={styles.cardHead}>
              <Typography.Text strong ellipsis className={styles.name} title={sessionLabel(row)}>
                {sessionLabel(row)}
              </Typography.Text>
              <Tag color={TONE_COLOR[statusTone(row.status_slug)]} title={statusLabel(row.status_slug)}>
                {statusLabel(row.status_slug)}
              </Tag>
            </div>
            <div className={styles.meta}>
              <span>{row.current_model ?? '模型未上报'}</span>
              <span>{relativeTime(row.last_active_unix)}</span>
              <span>{row.msg_count} 条</span>
              {row.pending_count > 0 ? <span>待执行 {row.pending_count}</span> : null}
            </div>
            {row.prompt_preview ? <p className={styles.preview}>{row.prompt_preview}</p> : null}
            <Space wrap className={styles.actions}>
              <Button size="small" onClick={() => onFocus(row.encoded_key)}>
                聚焦
              </Button>
              {canCreate ? (
                <>
                  <Button
                    size="small"
                    onClick={() => {
                      setRename(row)
                      setRenameValue(row.label ?? sessionLabel(row))
                    }}
                  >
                    重命名
                  </Button>
                  <Button size="small" danger onClick={() => setArchive(row)} disabled={busy}>
                    归档
                  </Button>
                </>
              ) : null}
            </Space>
          </Card>
        ))}
      </div>

      <Modal
        open={rename !== null}
        title="重命名会话"
        onCancel={() => setRename(null)}
        onOk={() => void commitRename()}
        confirmLoading={busy}
        okText="保存"
        cancelText="取消"
      >
        <Input value={renameValue} onChange={(event) => setRenameValue(event.target.value)} aria-label="会话名" />
      </Modal>

      <Modal
        open={archive !== null}
        title="归档会话"
        onCancel={() => setArchive(null)}
        onOk={() => void commitArchive()}
        confirmLoading={busy}
        okText="归档"
        okButtonProps={{ danger: true }}
        cancelText="取消"
      >
        <Typography.Paragraph>
          归档会终止子进程并移入历史，不可撤销。确定归档「
          {archive ? sessionLabel(archive) : ''}」？
        </Typography.Paragraph>
        {archive && archive.pending_count > 0 ? (
          <Alert
            type="warning"
            showIcon
            title={`该会话有 ${archive.pending_count} 条待执行提交，归档会一并丢弃。`}
          />
        ) : null}
      </Modal>
    </div>
  )
}
