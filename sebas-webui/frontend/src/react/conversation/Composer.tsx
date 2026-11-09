import { useEffect, useMemo, useRef, useState } from 'react'
import type { GetRef } from 'antd'
import { Alert, Button, Input, Select, Space, Tag, Typography } from 'antd'
import {
  api,
  errorText,
  NetworkError,
  type AvailableCommandInfo,
  type SessionDetail,
  type SessionUsage,
} from '../../api/client.js'
import {
  aliasEntryTitle,
  loadModelCatalog,
  mergeAliasEntries,
  type LoadedCatalog,
  type ModelAliasChoice,
} from '../../api/model-catalog.js'
import { notify } from '../../notify.js'
import { MODE_OPTIONS } from '../../views/mode-vocabulary.js'
import {
  completeCommand,
  filterCommands,
  messageForSubmit,
  submitFailureText,
  submitState,
  unsupportedSlashCommand,
  type SubmitState,
} from './composer-model.js'
import { renderMarkdown } from '../../components/markdown.js'
import styles from './composer.module.css'

export interface ComposerProps {
  sessionKey: string
  detail: SessionDetail
  reachability: { ok: boolean; cause?: string } | null
  onRefresh: () => void | Promise<void>
  send?: (key: string, text: string) => Promise<unknown>
  cancel?: (key: string) => Promise<unknown>
  setModel?: (key: string, model: string) => Promise<unknown>
  setMode?: (key: string, mode: string) => Promise<unknown>
  movePending?: (key: string, id: number, index: number) => Promise<unknown>
  removePending?: (key: string, id: number) => Promise<unknown>
  /** 别名来源：模型选择面并入别名短名（qa-round14 3.1）。 */
  loadCatalog?: () => Promise<LoadedCatalog>
}

const ACTION_LABEL: Record<SubmitState, string> = {
  disabled: '发送',
  send: '发送',
  sending: '发送中…',
  stop: '停止',
  stopping: '停止中…',
  queued: '排队发送',
  starting: '启动中…',
}

/** Session composer: send/stop/queue, model/mode, slash commands (4.1). */
export function Composer({
  sessionKey,
  detail,
  reachability,
  onRefresh,
  send = api.sendMessage,
  cancel = api.cancelSession,
  setModel = api.setSessionModel,
  setMode = api.setSessionMode,
  movePending = api.movePending,
  removePending = api.removePending,
  loadCatalog = loadModelCatalog,
}: ComposerProps) {
  const [text, setText] = useState('')
  const [sending, setSending] = useState(false)
  const [cancelling, setCancelling] = useState(false)
  const [switching, setSwitching] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const [paletteDismissed, setPaletteDismissed] = useState(false)
  const [highlight, setHighlight] = useState(0)
  const [aliases, setAliases] = useState<readonly ModelAliasChoice[]>([])
  const textarea = useRef<GetRef<typeof Input.TextArea>>(null)

  // Aliases are a first-class model value: fold them into the same selector
  // as the session's reported models (a name collision resolves to the alias).
  // A failed load simply leaves the plain model list.
  useEffect(() => {
    let cancelled = false
    loadCatalog()
      .then((loaded) => {
        if (!cancelled) setAliases(loaded.aliases)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [loadCatalog])

  const modelOptions = useMemo(() => {
    const entries = mergeAliasEntries(detail.available_models ?? [], aliases)
    return entries.map((entry) => {
      if (!entry.alias) return { value: entry.value, label: entry.value }
      const choice = aliases.find((item) => item.alias === entry.value)
      return {
        value: entry.value,
        label: `别名 / ${entry.value}`,
        title: choice ? aliasEntryTitle(choice) : undefined,
      }
    })
  }, [detail.available_models, aliases])

  const commands = detail.available_commands ?? []
  const filtered = useMemo(
    () => (paletteDismissed ? [] : filterCommands(commands, text)),
    [commands, text, paletteDismissed],
  )
  const state = submitState({
    text,
    sending,
    cancelling,
    turnEngaged: detail.turn_engaged,
    statusSlug: detail.status_slug,
  })
  const gated = reachability !== null && !reachability.ok

  async function removePendingItem(item: { id: number; text: string }): Promise<void> {
    try {
      await removePending(sessionKey, item.id)
      await onRefresh()
    } catch (failure) {
      // A deterministic 404 means the server truth still holds the entry; the
      // stack reconciles via onRefresh and the operator gets a named toast.
      notify({
        level: 'warn',
        message: `移除未生效：「${item.text}」——${errorText(failure)}`,
      })
      await onRefresh()
    }
  }

  async function submit(): Promise<void> {
    const message = messageForSubmit({ text, sending, cancelling, turnEngaged: detail.turn_engaged, statusSlug: detail.status_slug })
    if (message === null) return
    const unsupported = unsupportedSlashCommand(message, commands)
    if (unsupported !== null) {
      setNotice(unsupported)
      return
    }
    setSending(true)
    setError(null)
    setNotice(null)
    try {
      await send(sessionKey, message)
      setText('')
      setPaletteDismissed(false)
      await onRefresh()
    } catch (failure) {
      // A dead core (refused connection or a gateway 502 in front of it) keeps
      // the draft and names the failure kind — an opaque status would read as
      // an application rejection.
      setError(
        submitFailureText(failure, failure instanceof NetworkError, errorText(failure)),
      )
    } finally {
      setSending(false)
    }
  }

  async function stop(): Promise<void> {
    setCancelling(true)
    setError(null)
    try {
      await cancel(sessionKey)
      await onRefresh()
    } catch (failure) {
      setError(errorText(failure))
    } finally {
      setCancelling(false)
    }
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLTextAreaElement>): void {
    if (filtered.length > 0) {
      if (event.key === 'ArrowDown') {
        event.preventDefault()
        setHighlight((index) => (index + 1) % filtered.length)
        return
      }
      if (event.key === 'ArrowUp') {
        event.preventDefault()
        setHighlight((index) => (index - 1 + filtered.length) % filtered.length)
        return
      }
      if (event.key === 'Escape') {
        event.preventDefault()
        setPaletteDismissed(true)
        return
      }
      if (event.key === 'Tab' || (event.key === 'Enter' && !event.shiftKey && filtered.length > 0 && /^\/\S*$/.test(text))) {
        event.preventDefault()
        const command = filtered[Math.min(highlight, filtered.length - 1)]!
        setText(completeCommand(command))
        // Completion ends palette interaction: close it (the trailing space in
        // the completed text also closes it on the next keystroke) and hand
        // focus back to the textarea so arguments can be typed directly.
        setPaletteDismissed(true)
        setHighlight(0)
        textarea.current?.focus()
        return
      }
    }
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      void submit()
    }
  }

  return (
    <div className={styles.composer} data-testid="composer">
      {detail.pending.length > 0 ? (
        <div className={styles.queue} aria-label="待执行提交">
          {detail.pending.map((item, index) => (
            <div key={item.id} className={styles.queueRow}>
              <Tag>{index + 1}</Tag>
              <span className={styles.queueText}>{item.text}</span>
              <Space size={4}>
                <Button
                  size="small"
                  aria-label={`上移提交 ${index + 1}`}
                  disabled={index === 0}
                  onClick={() => void movePending(sessionKey, item.id, index - 1).then(onRefresh)}
                >
                  ↑
                </Button>
                <Button
                  size="small"
                  aria-label={`下移提交 ${index + 1}`}
                  disabled={index === detail.pending.length - 1}
                  onClick={() => void movePending(sessionKey, item.id, index + 1).then(onRefresh)}
                >
                  ↓
                </Button>
                <Button
                  size="small"
                  danger
                  aria-label={`移除提交 ${index + 1}`}
                  onClick={() => void removePendingItem(item)}
                >
                  移除
                </Button>
              </Space>
            </div>
          ))}
        </div>
      ) : null}

      {filtered.length > 0 ? (
        <ul className={styles.palette} role="listbox" aria-label="斜杠命令">
          {filtered.map((command: AvailableCommandInfo, index) => (
            <li key={command.name} role="option" aria-selected={index === highlight}>
              <button
                type="button"
                className={styles.paletteRow}
                onMouseEnter={() => setHighlight(index)}
                onClick={() => {
                  setText(completeCommand(command))
                  textarea.current?.focus()
                }}
              >
                <span className={styles.command}>/{command.name}</span>
                <span className={styles.hint}>{command.hint ?? ''}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
      {filtered.length > 0 ? (() => {
        // The highlighted row's description lives in a bounded detail bubble
        // rather than inline, so the palette stays one compact row per command
        // (workbench-composer-input-polish 3.1/3.2).
        const selected = filtered[Math.min(highlight, filtered.length - 1)]!
        return selected.description ? (
          <div
            className={styles.commandDetail}
            dangerouslySetInnerHTML={{ __html: renderMarkdown(selected.description) }}
          />
        ) : null
      })() : null}

      {notice !== null ? <Alert type="warning" showIcon title={notice} /> : null}
      {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
      {gated ? (
        <Alert
          type="warning"
          showIcon
          title={`核心不可达，提交已暂停${reachability?.cause ? `：${reachability.cause}` : ''}`}
        />
      ) : null}

      <Input.TextArea
        ref={textarea}
        aria-label="输入消息"
        value={text}
        rows={3}
        placeholder={detail.turn_engaged ? '回合进行中，发送会进入队列' : '输入消息，Enter 发送，Shift+Enter 换行'}
        onChange={(event) => {
          setText(event.target.value)
          setPaletteDismissed(false)
          setHighlight(0)
          setNotice(null)
        }}
        onKeyDown={onKeyDown}
      />

      <div className={styles.bar}>
        <Space wrap size={6}>
          <Tag>{detail.agent_kind ?? '默认 agent'}</Tag>
          <Select
            size="small"
            aria-label="权限模式"
            value={detail.desired_mode}
            options={MODE_OPTIONS.map((mode) => ({
              value: mode.value,
              label: mode.label,
              title: mode.description,
            }))}
            disabled={switching}
            onChange={(mode) => {
              setSwitching(true)
              setError(null)
              setMode(sessionKey, mode)
                .then(onRefresh)
                .catch((failure: unknown) => setError(errorText(failure)))
                .finally(() => setSwitching(false))
            }}
          />
          {modelOptions.length > 0 ? (
            <Select
              size="small"
              aria-label="会话模型"
              value={detail.current_model ?? undefined}
              placeholder="模型"
              options={modelOptions}
              disabled={switching}
              onChange={(model) => {
                setSwitching(true)
                setError(null)
                setModel(sessionKey, model)
                  .then(onRefresh)
                  .catch((failure: unknown) => setError(errorText(failure)))
                  .finally(() => setSwitching(false))
              }}
            />
          ) : detail.current_model ? (
            <Tag>{detail.current_model}</Tag>
          ) : (
            <Typography.Text type="secondary">模型未上报</Typography.Text>
          )}
          <UsageChip usage={detail.usage ?? null} />
        </Space>
        <Button
          type={state === 'stop' || state === 'stopping' ? 'default' : 'primary'}
          danger={state === 'stop'}
          data-state={state}
          disabled={state === 'disabled' || (gated && state !== 'stop')}
          loading={state === 'sending' || state === 'stopping'}
          onClick={() => {
            if (state === 'stop') void stop()
            else void submit()
          }}
        >
          {ACTION_LABEL[state]}
        </Button>
      </div>
    </div>
  )
}

function UsageChip({ usage }: { usage: SessionUsage | null }) {
  if (usage === null) return <Typography.Text type="secondary">token 未上报</Typography.Text>
  return (
    <Tag>
      入 {usage.total_input} · 出 {usage.total_output}
    </Tag>
  )
}
