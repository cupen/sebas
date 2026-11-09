import { useEffect, useState } from 'react'
import { Alert, Button, Input, Modal, Space, Spin, Typography } from 'antd'
import { api, errorText, type FsBrowseResponse } from '../../api/client.js'
import styles from './folder.module.css'

export function joinChildPath(parent: string, name: string): string {
  return `${parent.replace(/[\\/]+$/, '')}/${name}`
}

/** Same single-segment rule as the server-side mkdir. */
export function validateFolderName(name: string): string | null {
  const trimmed = name.trim()
  if (!trimmed) return '目录名不能为空'
  if (trimmed === '.' || trimmed === '..') return `目录名不能是「${trimmed}」`
  if (trimmed.includes('/') || trimmed.includes('\\')) return '目录名不能包含路径分隔符 / 或 \\'
  if (trimmed.includes('\0')) return '目录名包含非法字符'
  return null
}

/**
 * Map the server's project-registration boundary errors to a readable reason.
 * Only boundary/permission failures are shown as a blocking hint; the exact
 * server wording stays visible in the input-side alert.
 */
export function pathScopeHint(error: string): string | null {
  if (/超出允许范围|超出根目录范围|workspace root|outside the workspace root/.test(error)) {
    return '路径在 workspace root 之外——只能注册工作区内的目录'
  }
  if (/不存在|无法访问/.test(error)) {
    return '路径不存在或无法访问——请检查路径是否正确'
  }
  if (/不是目录/.test(error)) {
    return '该路径不是目录——请选择一个目录'
  }
  return null
}

export interface FolderPickerModalProps {
  open: boolean
  root?: string
  onClose: () => void
  onSelect: (path: string) => void
  browse?: (path: string, root?: string | null) => Promise<FsBrowseResponse>
  mkdir?: (path: string, name: string) => Promise<unknown>
}

/** Server-backed workspace directory picker (3.4). */
export function FolderPickerModal({
  open,
  root,
  onClose,
  onSelect,
  browse = api.fsBrowseDirs,
  mkdir = api.fsMkdir,
}: FolderPickerModalProps) {
  const [path, setPath] = useState('')
  const [rootPath, setRootPath] = useState('')
  const [entries, setEntries] = useState<FsBrowseResponse['entries']>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  const [newError, setNewError] = useState<string | null>(null)
  const [manual, setManual] = useState('')
  const [manualHint, setManualHint] = useState<string | null>(null)
  const [manualChecking, setManualChecking] = useState(false)

  async function list(target: string): Promise<void> {
    setLoading(true)
    setError(null)
    try {
      const response = await browse(target, root || undefined)
      setPath(response.path)
      if (!rootPath) setRootPath(response.path)
      setEntries(response.entries.filter((entry) => entry.is_dir))
    } catch (failure) {
      setError(errorText(failure))
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    if (!open) return
    setPath('')
    setRootPath('')
    setEntries([])
    setError(null)
    setNewName('')
    setNewError(null)
    setManual('')
    setManualHint(null)
    setManualChecking(false)
    setCreating(false)
    void list('')
    // Reset+load is keyed by open only; browse/root are stable props here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  useEffect(() => {
    if (!open) return
    const trimmed = manual.trim()
    if (trimmed === '') {
      setManualHint(null)
      setManualChecking(false)
      return
    }
    let cancelled = false
    setManualChecking(true)
    const timer = window.setTimeout(() => {
      browse(trimmed, root || undefined)
        .then(() => {
          if (!cancelled) {
            setManualHint(null)
            setManualChecking(false)
          }
        })
        .catch((failure: unknown) => {
          if (!cancelled) {
            setManualHint(pathScopeHint(errorText(failure)))
            setManualChecking(false)
          }
        })
    }, 300)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [manual, open, browse, root])

  const atRoot = rootPath !== '' && path === rootPath
  const parent = (() => {
    if (!path || atRoot) return ''
    const trimmed = path.replace(/[\\/]+$/, '')
    const cut = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
    return cut > 0 ? trimmed.slice(0, cut) : rootPath
  })()

  async function createFolder(): Promise<void> {
    const invalid = validateFolderName(newName)
    if (invalid !== null) {
      setNewError(invalid)
      return
    }
    setCreating(true)
    setNewError(null)
    try {
      await mkdir(path || rootPath, newName.trim())
      setNewName('')
      await list(path)
    } catch (failure) {
      setNewError(errorText(failure))
    } finally {
      setCreating(false)
    }
  }

  return (
    <Modal
      open={open}
      title="选择项目目录"
      onCancel={onClose}
      width={560}
      okText="选择此目录"
      cancelText="取消"
      okButtonProps={{ disabled: path === '' }}
      onOk={() => {
        if (path) onSelect(path)
      }}
    >
      <div className={styles.picker}>
        <div className={styles.bar}>
          <Button size="small" disabled={atRoot || !parent} onClick={() => void list(parent)}>
            上一级
          </Button>
          <Typography.Text code className={styles.path} title={path}>
            {path || '…'}
          </Typography.Text>
        </div>
        <div className={styles.manual}>
          <Input
            aria-label="项目路径"
            placeholder="或直接输入项目绝对路径"
            value={manual}
            status={manualHint !== null ? 'error' : undefined}
            onChange={(event) => {
              setManual(event.target.value)
              setManualHint(null)
            }}
            onPressEnter={() => {
              if (manual.trim() && manualHint === null) onSelect(manual.trim())
            }}
          />
          <Button
            loading={manualChecking}
            disabled={manual.trim() === '' || manualHint !== null}
            onClick={() => onSelect(manual.trim())}
          >
            使用此路径
          </Button>
        </div>
        {manualHint !== null ? <Alert type="error" showIcon title={manualHint} role="alert" /> : null}
        {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
        {loading ? (
          <Spin size="small" />
        ) : (
          <ul className={styles.list} aria-label="子目录">
            {entries.length === 0 ? <li className={styles.emptyItem}>没有子目录</li> : null}
            {entries.map((entry) => (
              <li key={entry.name} className={styles.item}>
                <button
                  type="button"
                  className={styles.itemButton}
                  onClick={() => void list(joinChildPath(path, entry.name))}
                >
                  <Space>
                    <span aria-hidden="true">▸</span>
                    <span>{entry.name}</span>
                  </Space>
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className={styles.mkdir}>
          <Input
            placeholder="新建文件夹名称"
            value={newName}
            onChange={(event) => setNewName(event.target.value)}
            onPressEnter={() => void createFolder()}
            aria-label="新建文件夹名称"
          />
          <Button onClick={() => void createFolder()} loading={creating}>
            新建
          </Button>
        </div>
        {newError !== null ? <Alert type="error" showIcon title={newError} role="alert" /> : null}
      </div>
    </Modal>
  )
}
