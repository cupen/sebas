import { useEffect, useMemo, useState } from 'react'
import { Alert, Form, Input, Modal, Select } from 'antd'
import { api, errorText, MODEL_SLOTS, type AgentKindInfo, type Project } from '../../api/client.js'
import { notify } from '../../notify.js'
import { MODE_OPTIONS } from '../../views/mode-vocabulary.js'
import {
  loadLastUsedPair,
  loadModelCatalog,
  preselectLastUsed,
  saveLastUsedPair,
  type LoadedCatalog,
} from '../../api/model-catalog.js'
import styles from './session-create.module.css'

const MODES = MODE_OPTIONS.map((mode) => ({
  value: mode.value,
  label: mode.label,
  title: mode.label,
}))

export interface CreateSessionDraft {
  projectId: string | null
  agent: string | null
  prompt: string
  mode: string | null
  model: string | null
  /** 可选预命名（qa-round14 4.7）：创建后立即作为会话 label 写回。 */
  title?: string
}

/** Client-side gate before `POST /api/sessions`; null means submit. */
export function createDraftError(draft: CreateSessionDraft): string | null {
  if (draft.projectId === null) return '请选择项目'
  if (draft.agent === null) return '请选择 agent'
  return null
}

/** Keep the existing wire shape: agent/project required, model+mode optional. */
export function createSessionPayload(draft: CreateSessionDraft): {
  agent: string
  projectId: string
  prompt: string | null
  model: string | null
  mode: string | null
} | null {
  if (createDraftError(draft) !== null) return null
  return {
    agent: draft.agent!,
    projectId: draft.projectId!,
    prompt: draft.prompt.trim() ? draft.prompt : null,
    model: draft.model,
    mode: draft.mode,
  }
}

export interface SessionCreateModalProps {
  open: boolean
  projectId?: string | null
  projects: Project[]
  onClose: () => void
  onCreated: (encodedKey: string) => void
  loadAgents?: () => Promise<{ agents: AgentKindInfo[] }>
  loadCatalog?: () => Promise<LoadedCatalog>
  createSession?: typeof api.createSession
  /** 带 title 创建后的兜底重命名链（走既有 label 写面）。 */
  renameSession?: typeof api.setSessionLabel
}

/** Real session creation against `POST /api/sessions` (3.5). */
export function SessionCreateModal({
  open,
  projectId,
  projects,
  onClose,
  onCreated,
  loadAgents = api.agents,
  loadCatalog = loadModelCatalog,
  createSession = api.createSession,
  renameSession = api.setSessionLabel,
}: SessionCreateModalProps) {
  const [selectedProject, setSelectedProject] = useState<string | null>(projectId ?? null)
  const [agent, setAgent] = useState<string | null>(null)
  const [mode, setMode] = useState<string | null>('ask')
  const [model, setModel] = useState<string | null>(null)
  const [provider, setProvider] = useState<string | null>(null)
  const [catalog, setCatalog] = useState<LoadedCatalog | null>(null)
  const [prompt, setPrompt] = useState('')
  const [title, setTitle] = useState('')
  const [agents, setAgents] = useState<AgentKindInfo[]>([])
  const [agentsError, setAgentsError] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    if (!open) return
    setSelectedProject(projectId ?? null)
    setAgent(null)
    setMode('ask')
    setModel(null)
    setProvider(null)
    setCatalog(null)
    setPrompt('')
    setTitle('')
    setError(null)
    setBusy(false)
    setAgentsError(null)
    let cancelled = false
    loadAgents()
      .then((response) => {
        if (!cancelled) setAgents(response.agents)
      })
      .catch((failure: unknown) => {
        if (!cancelled) setAgentsError(errorText(failure))
      })
    loadCatalog()
      .then((loaded) => {
        if (cancelled) return
        setCatalog(loaded)
        if (loaded.catalog !== null) {
          const preselected = preselectLastUsed(loaded.catalog, loadLastUsedPair())
          setProvider(preselected.provider)
          setModel(preselected.model)
        }
      })
      .catch(() => {
        /* catalog is optional: creation still works with the agent default */
      })
    return () => {
      cancelled = true
    }
  }, [open, projectId, loadAgents])

  const agentOptions = useMemo(
    () =>
      agents.map((item) => ({
        value: item.id,
        label: item.reachable ? item.display : `${item.display}（不可达：${item.cause ?? '未知原因'}）`,
        disabled: !item.reachable,
      })),
    [agents],
  )

  const modelOptions = useMemo(() => {
    if (catalog === null) return []
    const pairs = (catalog.catalog?.pairs ?? []).map((pair) => ({
      value: pair.model,
      label: `${pair.provider} / ${pair.model}`,
    }))
    const aliases = catalog.aliases.map((entry) => ({
      value: entry.alias,
      label: `别名 / ${entry.alias}`,
    }))
    return [...pairs, ...aliases]
  }, [catalog])

  const selectedAgent = agents.find((item) => item.id === agent) ?? null
  const slotMapping = selectedAgent?.models ?? null
  const slotOptions = useMemo(() => {
    if (slotMapping === null) return []
    return [
      { value: 'default', label: 'default' },
      ...MODEL_SLOTS.filter((slot) => slotMapping[slot]).map((slot) => ({
        value: slot,
        label: slot,
      })),
    ]
  }, [slotMapping])

  function chooseAgent(value: string): void {
    setAgent(value)
    const chosen = agents.find((item) => item.id === value)
    if (chosen?.models) {
      // A slot already chosen for this agent stays valid; only fall back to
      // the mapping's default when the current value is not an enabled slot.
      const currentIsSlot =
        model !== null && MODEL_SLOTS.some((slot) => slot === model && Boolean(chosen.models?.[slot]))
      if (currentIsSlot) return
      const saved = chosen.models.default
      setModel(saved && chosen.models[saved] ? saved : null)
      setProvider(null)
    } else if (catalog?.catalog) {
      // Keep an explicit pair the user already picked in this dialog; the
      // catalog preselection only fills an empty/invalid choice.
      const currentIsPair =
        provider !== null &&
        model !== null &&
        catalog.catalog.pairs.some(
          (pair) => pair.provider === provider && pair.model === model,
        )
      if (currentIsPair) return
      const preselected = preselectLastUsed(catalog.catalog, loadLastUsedPair())
      setProvider(preselected.provider)
      setModel(preselected.model)
    } else {
      setModel(null)
      setProvider(null)
    }
  }

  async function submit(): Promise<void> {
    if (busy) return
    const draft: CreateSessionDraft = { projectId: selectedProject, agent, prompt, mode, model }
    const invalid = createDraftError(draft)
    if (invalid !== null) {
      setError(invalid)
      return
    }
    const payload = createSessionPayload(draft)
    if (payload === null) return
    setBusy(true)
    setError(null)
    try {
      const response = await createSession(payload)
      const known =
        provider !== null &&
        model !== null &&
        catalog?.catalog?.pairs.some(
          (candidate) => candidate.provider === provider && candidate.model === model,
        ) === true
      if (known) saveLastUsedPair({ provider, model })
      // Optional title: rename right after creation (the rail row shows it
      // without a separate manual rename step). A failed rename must not lose
      // the created session — surface it and continue into the session.
      const wanted = title.trim()
      if (wanted !== '') {
        try {
          await renameSession(response.key, wanted)
        } catch (failure) {
          notify({
            level: 'warn',
            message: `会话已创建，但标题「${wanted}」未生效：${errorText(failure)}`,
          })
        }
      }
      onCreated(response.key)
    } catch (failure) {
      // One alert only: the dialog's inline error is the action-point notice.
      // A duplicate toast would also make the rejection ambiguous to a11y.
      setError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Modal
      open={open}
      title={<h2 className={styles.dialogTitle}>新建会话</h2>}
      onCancel={onClose}
      onOk={() => void submit()}
      okText="创建"
      cancelText="取消"
      confirmLoading={busy}
      width={560}
    >
      <Form layout="vertical" className={styles.form} onFinish={() => void submit()}>
        <Form.Item label="项目" required>
          <Select
            aria-label="项目"
            value={selectedProject ?? undefined}
            placeholder="选择项目"
            options={projects.map((project) => ({ value: project.id, label: project.name }))}
            onChange={(value) => setSelectedProject(value)}
          />
        </Form.Item>
        <Form.Item label="标题（可选）">
          <Input
            aria-label="标题"
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            placeholder="给这次会话起个名字（可留空）"
          />
        </Form.Item>
        <Form.Item label="Agent" required>
          <Select
            aria-label="Agent"
            value={agent ?? undefined}
            placeholder="选择 agent"
            options={agentOptions}
            onChange={chooseAgent}
          />
        </Form.Item>
        <Form.Item label="权限模式">
          <Select
            aria-label="权限模式"
            value={mode ?? undefined}
            options={MODES}
            onChange={(value) => setMode(value)}
          />
        </Form.Item>
        {slotMapping !== null ? (
          <Form.Item label="模型档位">
            <Select
              aria-label="模型档位"
              value={model ?? undefined}
              options={slotOptions}
              onChange={(value) => {
                setModel(value)
                setProvider(null)
              }}
            />
          </Form.Item>
        ) : (
          <Form.Item
            label="模型（可选）"
            extra={catalog?.unavailable ? '模型目录不可用，将使用 agent 默认模型' : undefined}
          >
            <Select
              aria-label="模型"
              allowClear
              showSearch
              value={model ?? undefined}
              placeholder={catalog?.unavailable ? '模型目录不可用' : '使用 agent 默认模型'}
              options={modelOptions}
              disabled={modelOptions.length === 0}
              onChange={(value) => {
                setModel(value ?? null)
                setProvider(
                  catalog?.catalog?.pairs.find((candidate) => candidate.model === value)?.provider ??
                    null,
                )
              }}
            />
          </Form.Item>
        )}
        <Form.Item label="任务描述">
          <Input.TextArea
            aria-label="任务描述"
            rows={3}
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            placeholder="描述这次会话要完成的工作（可留空）"
          />
        </Form.Item>
        {agentsError !== null ? <Alert type="warning" showIcon title={`Agent 目录加载失败：${agentsError}`} /> : null}
        {error !== null ? <Alert type="error" showIcon title={error} role="alert" /> : null}
      </Form>
    </Modal>
  )
}
