import { useEffect, useMemo, useState } from 'react'
import {
  Alert,
  Button,
  Dropdown,
  Empty,
  Input,
  Modal,
  Spin,
  Tag,
  Typography,
} from 'antd'
import {
  api,
  errorText,
  type ArchiveDetail,
  type ArchiveEntry,
  type Project,
  type SessionRow,
} from '../../api/client.js'
import {
  readRailExpanded,
  readRailHistoryOpen,
  readSeenAnchor,
  writeRailExpanded,
  writeRailHistoryOpen,
  writeSeenAnchor,
} from '../boundary/preferences.js'
import { ANCHOR_ADVANCED_EVENT } from '../../views/unread-cursor.js'
import { notify } from '../../notify.js'
import { sharedWs } from '../../api/shared-ws.js'
import type { SessionStore } from '../state/session-store.js'
import { useStore } from '../state/use-store.js'
import { sessionLabel, statusLabel } from './labels.js'
import { FolderPickerModal } from './FolderPickerModal.js'
import styles from './rail.module.css'

export interface ProjectRailProps {
  sessions: SessionStore
  selectedProjectId: string | null
  onSelectProject: (id: string | null) => void
  canCreate: boolean
  focusedKey?: string | null
  onFocus: (key: string) => void
  onRequestCreate: (projectId: string) => void
  /** Transport is down: the tree is stale — say so instead of faking freshness. */
  disconnected?: boolean
  onRetry?: () => void
}

function unreadCount(row: SessionRow, focusedKey: string | null): number {
  if (!row.msg_count) return 0
  const anchor = readSeenAnchor(row.encoded_key)
  if (anchor === null) return 0
  const unread = Math.max(0, row.msg_count - anchor)
  // Lit parity (round3 6.1): the badge's subject is an arrival in a session the
  // operator is NOT looking at. While the focused row's transcript is on screen
  // a visible-to-the-eye arrival must not be marked unread — the transcript's
  // follow-to-bottom path advances the same anchor. A background tab keeps the
  // badge (hidden does not count as seeing).
  if (unread > 0 && row.encoded_key === focusedKey && document.visibilityState === 'visible') {
    return 0
  }
  return unread
}

export function ProjectRail({
  sessions,
  selectedProjectId,
  onSelectProject,
  canCreate,
  focusedKey = null,
  onFocus,
  onRequestCreate,
  disconnected = false,
  onRetry,
}: ProjectRailProps) {
  const projects = useStore(sessions.store, (state) => state.projects)
  const list = useStore(sessions.store, (state) => state.list)
  const loading = useStore(sessions.store, (state) => state.loading)
  const [archive, setArchive] = useState<ArchiveEntry[]>([])
  const [archiveError, setArchiveError] = useState<string | null>(null)
  const [archiveOpen, setArchiveOpen] = useState(readRailHistoryOpen)
  const [folderOpen, setFolderOpen] = useState(false)
  const [addError, setAddError] = useState<string | null>(null)
  const [renameProject, setRenameProject] = useState<Project | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const [removeProject, setRemoveProject] = useState<Project | null>(null)
  const [sessionRename, setSessionRename] = useState<SessionRow | null>(null)
  const [sessionName, setSessionName] = useState('')
  const [sessionArchive, setSessionArchive] = useState<SessionRow | null>(null)
  const [archiveDetail, setArchiveDetail] = useState<ArchiveDetail | null>(null)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [expanded, setExpanded] = useState<Record<string, boolean>>(readRailExpanded)
  // localStorage is not reactive: transcript/focus anchor advances dispatch
  // ANCHOR_ADVANCED_EVENT so badges re-read the shared cursor without waiting
  // for an unrelated list refresh (round3 6.1).
  const [anchorTick, setAnchorTick] = useState(0)

  const reloadArchive = async (): Promise<void> => {
    try {
      const response = await api.archiveList()
      // Newest archive first (rail-declutter-unread D7); equal timestamps keep
      // the server's insertion order via a stable sort.
      setArchive(
        [...response.archived_sessions].sort((a, b) => b.archived_at - a.archived_at),
      )
      setArchiveError(null)
    } catch (failure) {
      setArchiveError(errorText(failure))
    }
  }

  useEffect(() => {
    void reloadArchive()
  }, [])

  // Archive is a server-side fact: an archive performed elsewhere (API/another
  // client) arrives as session.removed and must move the history badge/count
  // without a reload (round14 4.5 / round10 badge parity).
  useEffect(() => {
    return sharedWs.subscribe((event) => {
      if (event.type === 'session.removed') void reloadArchive()
    })
  }, [])

  const rows = list?.recent_sessions ?? []

  useEffect(() => {
    const onAdvanced = (event: Event): void => {
      const key = (event as CustomEvent<{ key?: unknown }>).detail?.key
      if (typeof key === 'string' && rows.some((row) => row.encoded_key === key)) {
        setAnchorTick((tick) => tick + 1)
      }
    }
    window.addEventListener(ANCHOR_ADVANCED_EVENT, onAdvanced)
    return () => window.removeEventListener(ANCHOR_ADVANCED_EVENT, onAdvanced)
  }, [rows])

  // First sight of a session establishes its baseline: no stored anchor reads
  // as fully read (spec「无锚 = 全部已读」), so the current count becomes the
  // starting point and only later arrivals can light the badge. Without this,
  // sessions created outside the rail (API/another client) could never badge.
  useEffect(() => {
    for (const row of rows) {
      if (readSeenAnchor(row.encoded_key) === null) {
        writeSeenAnchor(row.encoded_key, row.msg_count)
      }
    }
  }, [rows])

  const unreadByKey = useMemo(() => {
    const counts = new Map<string, number>()
    for (const row of rows) counts.set(row.encoded_key, unreadCount(row, focusedKey))
    return counts
    // anchorTick participates so an external anchor advance re-reads storage.
  }, [rows, anchorTick, focusedKey])

  const grouped = useMemo(() => {
    const byProject = new Map<string, SessionRow[]>()
    for (const row of rows) {
      const key = row.project_id ?? '__inbox__'
      const bucket = byProject.get(key) ?? []
      bucket.push(row)
      byProject.set(key, bucket)
    }
    return byProject
  }, [rows])

  async function addProject(path: string): Promise<void> {
    setBusy(true)
    setAddError(null)
    try {
      await api.projects.add(path)
      setFolderOpen(false)
      await sessions.refresh()
    } catch (failure) {
      setAddError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function commitRenameProject(): Promise<void> {
    if (renameProject === null) return
    if (!renameValue.trim()) {
      setActionError('项目名不能为空')
      return
    }
    setBusy(true)
    setActionError(null)
    try {
      await api.projects.rename(renameProject.id, renameValue.trim())
      setRenameProject(null)
      await sessions.refresh()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function commitRemoveProject(): Promise<void> {
    if (removeProject === null) return
    setBusy(true)
    setActionError(null)
    try {
      await api.projects.remove(removeProject.id)
      setRemoveProject(null)
      if (selectedProjectId === removeProject.id) onSelectProject(null)
      await sessions.refresh()
      notify({
        level: 'info',
        message: `项目「${removeProject.name}」已移除注册，可重新添加。`,
      })
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function moveProject(project: Project, delta: -1 | 1): Promise<void> {
    const index = projects.findIndex((item) => item.id === project.id)
    const target = index + delta
    if (index < 0 || target < 0 || target >= projects.length) return
    const ids = projects.map((item) => item.id)
    const current = ids[index]!
    ids[index] = ids[target]!
    ids[target] = current
    setBusy(true)
    setActionError(null)
    try {
      await api.projects.reorder(ids)
      await sessions.refresh()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function commitSessionRename(): Promise<void> {
    if (sessionRename === null) return
    setBusy(true)
    setActionError(null)
    try {
      await api.setSessionLabel(sessionRename.encoded_key, sessionName.trim() || null)
      setSessionRename(null)
      await sessions.refresh()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function commitSessionArchive(): Promise<void> {
    if (sessionArchive === null) return
    setBusy(true)
    setActionError(null)
    try {
      await api.archiveSession(sessionArchive.encoded_key)
      setSessionArchive(null)
      await sessions.refresh()
      await reloadArchive()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function closeSession(row: SessionRow): Promise<void> {
    setBusy(true)
    setActionError(null)
    try {
      await api.closeSession(row.encoded_key)
      await sessions.refresh()
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function openArchive(entry: ArchiveEntry): Promise<void> {
    setBusy(true)
    setActionError(null)
    try {
      setArchiveDetail(await api.archiveDetail(entry.session_key))
    } catch (failure) {
      setActionError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  async function restoreArchive(entry: ArchiveEntry): Promise<void> {
    setBusy(true)
    setActionError(null)
    const label = entry.operator_label || entry.label
    try {
      await api.restoreSession(entry.session_key)
      setArchiveDetail(null)
      await sessions.refresh()
      await reloadArchive()
      notify({ level: 'info', message: `会话「${label}」已恢复。` })
    } catch (failure) {
      const message = errorText(failure)
      setActionError(message)
      notify({ level: 'error', message: `恢复会话「${label}」失败：${message}` })
    } finally {
      setBusy(false)
    }
  }

  const projectSessions = (project: Project): SessionRow[] =>
    grouped.get(project.id) ?? []

  /**
   * Same rule as the Lit rail: a persisted toggle wins; otherwise the
   * project holding the focused session opens by default so a deep link never
   * hides the row the operator is reading.
   */
  function isExpanded(project: Project): boolean {
    const recorded = expanded[project.path]
    if (recorded !== undefined) return recorded
    // No focus (the overview/home shell) shows the whole tree; a deep link
    // opens only the project being read. A recorded toggle always wins.
    if (focusedKey === null) return true
    return grouped.get(project.id)?.some((row) => row.encoded_key === focusedKey) === true
  }

  function toggleProject(project: Project): void {
    const next = { ...expanded, [project.path]: !isExpanded(project) }
    setExpanded(next)
    writeRailExpanded(next)
    onSelectProject(project.id)
  }

  return (
    <div className={styles.rail}>
      <div className={styles.sectionHead}>
        <span className={styles.sectionTitle}>项目</span>
        {canCreate ? (
          <Button size="small" type="text" onClick={() => setFolderOpen(true)}>
            添加项目
          </Button>
        ) : null}
      </div>
      {addError !== null ? <Alert type="error" showIcon title={addError} role="alert" /> : null}
      {disconnected ? (
        // Honest degradation (singleprocess-dead-core): with the transport down
        // the cached tree is not fresh. Name it and offer the retry instead of
        // rendering a stale list as if nothing happened.
        <div className={styles.unreachable} role="status">
          <span>无法连接服务器</span>
          {onRetry ? (
            <Button size="small" type="text" onClick={onRetry}>
              重试
            </Button>
          ) : null}
        </div>
      ) : null}
      {loading && projects.length === 0 ? <Spin size="small" /> : null}
      {projects.length === 0 && !loading ? <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有项目" /> : null}

      <div className={styles.projects}>
        {projects.map((project, index) => {
          const projectRows = projectSessions(project)
          const active = project.id === selectedProjectId
          const open = isExpanded(project)
          return (
            <section key={project.id} className={styles.project}>
              <div className={`${styles.projectRow} ${active ? styles.projectActive : ''}`}>
                <button
                  type="button"
                  className={styles.projectButton}
                  aria-label={`${project.name} ${projectRows.length}`}
                  onClick={() => toggleProject(project)}
                  aria-pressed={active}
                  aria-expanded={open}
                >
                  <span className={styles.projectName}>{project.name}</span>
                  <span className={styles.count}>{projectRows.length}</span>
                </button>
                {canCreate ? (
                  <Dropdown
                    trigger={['click']}
                    menu={{
                      items: [
                        { key: 'new', label: '新建会话' },
                        { key: 'rename', label: '重命名项目' },
                        {
                          key: 'up',
                          label: '上移项目',
                          disabled: index === 0,
                        },
                        {
                          key: 'down',
                          label: '下移项目',
                          disabled: index === projects.length - 1,
                        },
                        { key: 'remove', label: '移除项目', danger: true },
                      ],
                      onClick: ({ key, domEvent }) => {
                        domEvent.stopPropagation()
                        if (key === 'new') {
                          onSelectProject(project.id)
                          onRequestCreate(project.id)
                        } else if (key === 'rename') {
                          setRenameProject(project)
                          setRenameValue(project.name)
                        } else if (key === 'up') {
                          void moveProject(project, -1)
                        } else if (key === 'down') {
                          void moveProject(project, 1)
                        } else {
                          setRemoveProject(project)
                        }
                      },
                    }}
                  >
                    <Button size="small" type="text" aria-label={`项目操作 ${project.name}`}>
                      …
                    </Button>
                  </Dropdown>
                ) : null}
              </div>
              <ul className={styles.sessions} hidden={!open}>
                {projectRows.map((row) => (
                  <li key={row.encoded_key}>
                    <div className={styles.sessionRow}>
                      <button
                        type="button"
                        className={styles.sessionButton}
                        aria-label={sessionLabel(row)}
                        onClick={() => onFocus(row.encoded_key)}
                      >
                        <span
                          className={`${styles.dot} ${styles[`dot_${row.status_slug}`] ?? ''}`}
                          aria-hidden="true"
                        />
                        <span className={styles.sessionText}>
                          <span className={styles.sessionName}>{sessionLabel(row)}</span>
                          <span className={styles.sessionMeta}>
                            {statusLabel(row.status_slug)} · {row.prompt_preview ?? '无预览'}
                          </span>
                        </span>
                        {(unreadByKey.get(row.encoded_key) ?? 0) > 0 ? (
                          <Tag color="orange" className={styles.unread}>
                            {unreadByKey.get(row.encoded_key)}
                          </Tag>
                        ) : null}
                      </button>
                      <Dropdown
                        trigger={['click']}
                        menu={{
                          items: [
                            { key: 'rename', label: '重命名会话' },
                            { key: 'close', label: '关闭会话' },
                            { key: 'archive', label: '归档会话', danger: true },
                          ],
                          onClick: ({ key, domEvent }) => {
                            domEvent.stopPropagation()
                            if (key === 'rename') {
                            setSessionRename(row)
                            // Only a real label is editable text; an unlabelled
                            // row starts empty so typing replaces rather than
                            // appending to the prompt preview (Lit parity).
                            setSessionName(row.label ?? '')
                            } else if (key === 'close') {
                              void closeSession(row)
                            } else {
                              setSessionArchive(row)
                            }
                          },
                        }}
                      >
                        <Button size="small" type="text" aria-label={`会话操作 ${sessionLabel(row)}`}>
                          …
                        </Button>
                      </Dropdown>
                    </div>
                  </li>
                ))}
              </ul>
            </section>
          )
        })}
      </div>

      <div className={styles.archive}>
        <button
          type="button"
          className={styles.archiveToggle}
          aria-expanded={archiveOpen}
          onClick={() =>
            setArchiveOpen((value) => {
              writeRailHistoryOpen(!value)
              return !value
            })
          }
        >
          <span>历史</span>
          <span className={styles.count}>{archive.length}</span>
        </button>
        {archiveOpen ? (
          <div className={styles.archiveList}>
            {archiveError !== null ? <Alert type="error" showIcon title={archiveError} /> : null}
            {archive.length === 0 ? <span className={styles.empty}>没有归档会话</span> : null}
            {archive.map((entry) => (
              <div key={entry.session_key} className={styles.archiveRow}>
                <button type="button" onClick={() => void openArchive(entry)}>
                  <span className={styles.sessionName}>{entry.operator_label || entry.label}</span>
                  <span className={styles.sessionMeta}>
                    归档于 {new Date(entry.archived_at * 1000).toLocaleDateString('zh-CN')}
                  </span>
                </button>
                <Button
                  size="small"
                  aria-label={`恢复会话 ${entry.operator_label || entry.label}`}
                  onClick={() => void restoreArchive(entry)}
                  loading={busy}
                >
                  恢复
                </Button>
              </div>
            ))}
          </div>
        ) : null}
      </div>

      <FolderPickerModal
        open={folderOpen}
        onClose={() => setFolderOpen(false)}
        onSelect={(path) => void addProject(path)}
      />

      <Modal
        open={renameProject !== null}
        title="重命名项目"
        onCancel={() => setRenameProject(null)}
        onOk={() => void commitRenameProject()}
        confirmLoading={busy}
        okText="保存"
        cancelText="取消"
      >
        <Input value={renameValue} onChange={(event) => setRenameValue(event.target.value)} aria-label="项目名" />
        {actionError !== null ? <Alert type="error" showIcon title={actionError} /> : null}
      </Modal>

      <Modal
        open={removeProject !== null}
        title="移除项目"
        onCancel={() => setRemoveProject(null)}
        onOk={() => void commitRemoveProject()}
        confirmLoading={busy}
        okText="移除"
        okButtonProps={{ danger: true }}
        cancelText="取消"
      >
        <Typography.Paragraph>
          移除「{removeProject?.name}」只影响项目注册表，不会删除磁盘目录。
        </Typography.Paragraph>
        {actionError !== null ? <Alert type="error" showIcon title={actionError} /> : null}
      </Modal>

      <Modal
        open={sessionRename !== null}
        title="重命名会话"
        onCancel={() => setSessionRename(null)}
        onOk={() => void commitSessionRename()}
        confirmLoading={busy}
        okText="保存"
        cancelText="取消"
      >
        <Input value={sessionName} onChange={(event) => setSessionName(event.target.value)} aria-label="会话名" />
        <Typography.Paragraph type="secondary" className={styles.hint}>
          留空即回退到首条消息预览。
        </Typography.Paragraph>
        {actionError !== null ? <Alert type="error" showIcon title={actionError} /> : null}
      </Modal>

      <Modal
        open={sessionArchive !== null}
        title="归档会话"
        onCancel={() => setSessionArchive(null)}
        onOk={() => void commitSessionArchive()}
        confirmLoading={busy}
        okText="归档"
        okButtonProps={{ danger: true }}
        cancelText="取消"
      >
        <Typography.Paragraph>
          归档会终止子进程并移入历史，不可撤销。确定归档「
          {sessionArchive ? sessionLabel(sessionArchive) : ''}」？
        </Typography.Paragraph>
        {sessionArchive && sessionArchive.pending_count > 0 ? (
          <Alert
            type="warning"
            showIcon
            title={`该会话有 ${sessionArchive.pending_count} 条待执行提交，归档会一并丢弃。`}
          />
        ) : null}
        {actionError !== null ? <Alert type="error" showIcon title={actionError} /> : null}
      </Modal>

      <Modal
        open={archiveDetail !== null}
        title={archiveDetail ? `归档会话：${archiveDetail.entry.operator_label || archiveDetail.entry.label}` : ''}
        onCancel={() => setArchiveDetail(null)}
        footer={
          archiveDetail ? (
            <Button danger onClick={() => void restoreArchive(archiveDetail.entry)} loading={busy}>
              恢复会话
            </Button>
          ) : null
        }
        width={720}
      >
        <div className={styles.archiveDetail}>
          {archiveDetail?.entries.length === 0 ? <Empty description="归档快照为空" /> : null}
          {archiveDetail?.entries.map((entry) => (
            <div key={entry.position} className={styles.archiveEntry}>
              <span className={styles.sessionMeta}>
                {entry.kind === 'prompt' ? '你' : 'agent'} · {entry.element_type}
              </span>
              <pre className={styles.archiveText}>{entry.content}</pre>
            </div>
          ))}
        </div>
        {actionError !== null ? <Alert type="error" showIcon title={actionError} /> : null}
      </Modal>
    </div>
  )
}
