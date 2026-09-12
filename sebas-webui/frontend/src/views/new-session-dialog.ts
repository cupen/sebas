/**
 * New-session creation dialog (workbench-interaction-polish D2, rail 持有).
 *
 * The ONLY place an agent can be chosen (agent is immutable after creation).
 * wa-dialog 承载：agent 必选下拉（/api/agents，预选项目 default_agent，
 * 无记录时首个可达 agent）+ 两级模型选择（provider → model，Settings 目录
 * 共用 loadModelCatalog；目录不可得显式说明）+ 权限 mode 下拉
 * （ask | edit | allow | auto，缺省「agent 默认」= wire 上省略 mode 字段）。
 *
 * Confirm dispatches `dialog-confirm` with { agent, model, mode }; the rail
 * performs the POST /api/sessions call and the post-create focus flow.
 * Cancel dispatches `dialog-cancel` — nothing is created, nothing focused.
 * 无可选 agent（catalog 为空）时确认钮禁用（spec「dialog requires an
 * explicit agent」）。
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type AgentKindInfo } from '../api/client.js'
import {
  loadModelCatalog,
  preselectFromCatalog,
  type ModelCatalog,
} from '../api/model-catalog.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

/** 创建对话框确认事件 detail：与 POST /api/sessions 的创建面同词汇。 */
export interface NewSessionDialogConfirm {
  agent: string
  /** 目录选定的模型 id；`null` = 目录不可得或未选（创建请求省略）。 */
  model: string | null
  /** `null` = agent 默认（wire 上省略 mode 字段）。 */
  mode: string | null
}

@customElement('sebas-new-session-dialog')
export class SebasNewSessionDialog extends LitElement {
  /** 开合（rail 持有；项目行「+」打开）。 */
  @property({ type: Boolean }) open = false
  /** 目标项目的稳定 id（创建请求的 project_id；`null` 不携带）。 */
  @property({ attribute: false }) projectId: string | null = null
  /** 项目名（对话框标题语境）。 */
  @property({ attribute: false }) projectName: string | null = null
  /** 项目记住的 default agent（预选；`null` = 首访，兜底首个可达 agent）。 */
  @property({ attribute: false }) defaultAgent: string | null = null
  /** 创建失败的就地呈现（rail 写入；开盒/成功时清空）。 */
  @property({ attribute: false }) error: string | null = null

  /** Agent catalog（/api/agents；唯一可用性真源，含 native 行）。 */
  @state() private agents: AgentKindInfo[] = []
  /** 选定的 agent id（必填；空 = 未选 → 确认禁用）。 */
  @state() private agent = ''
  /** 两级选择的一级：provider 名。 */
  @state() private selectedProvider: string | null = null
  /** 两级选择的二级：模型 id。 */
  @state() private model: string | null = null
  /** Settings 目录；`null` = 目录尚未取得或不可得。 */
  @state() private catalog: ModelCatalog | null = null
  /** 目录显式不可得（空目录或读取失败）：就地说明，不渲染空下拉。 */
  @state() private catalogUnavailable = false
  /** 选定的权限模式；`null` = agent 默认（缺省项，不发送 mode 字段）。 */
  @state() private mode: string | null = null

  /** 一级选定的 provider 下的模型 id 列表（保持 payload 顺序）。 */
  private modelsFor(provider: string): string[] {
    return this.catalog?.pairs.filter((p) => p.provider === provider).map((p) => p.model) ?? []
  }

  /** 两级目录去重后的 provider 列表。 */
  private get catalogProviders(): string[] {
    const out: string[] = []
    for (const p of this.catalog?.pairs ?? []) {
      if (!out.includes(p.provider)) out.push(p.provider)
    }
    return out
  }

  private reloadAgentsBound = (): void => {
    void this.loadAgents()
    void this.loadCatalog()
  }

  connectedCallback(): void {
    super.connectedCallback()
    void this.loadAgents()
    void this.loadCatalog()
    // catalog/defaults 变更（管理页）即时反映——不要求重启。
    window.addEventListener('sebas:refetch', this.reloadAgentsBound)
  }

  disconnectedCallback(): void {
    window.removeEventListener('sebas:refetch', this.reloadAgentsBound)
    super.disconnectedCallback()
  }

  protected willUpdate(changed: PropertyValues): void {
    // 打开时重置表单：agent 预选项目 default_agent（无记录兜底首个可达），
    // 模型预选目录 default；mode 回到「agent 默认」。重取数据源（管理页
    // 可能刚改过目录/defaults）。
    if (changed.has('open') && this.open) {
      void this.loadAgents()
      void this.loadCatalog()
      this.mode = null
      this.agent = ''
      if (this.defaultAgent) this.agent = this.defaultAgent
      this.applyCatalogPreselect()
    }
    // 项目切换（rail 对不同项目的「+」复用同一对话框实例）。
    if (changed.has('defaultAgent') && this.open && this.defaultAgent) {
      this.agent = this.defaultAgent
    }
  }

  /** 目录预选规则（共用 preselectFromCatalog，default 不在目录内不伪造）。 */
  private applyCatalogPreselect(): void {
    if (!this.catalog) {
      this.selectedProvider = null
      this.model = null
      return
    }
    const pre = preselectFromCatalog(this.catalog)
    this.selectedProvider = pre.provider
    this.model = pre.model
  }

  private async loadAgents(): Promise<void> {
    try {
      const data = await api.agents()
      this.agents = data.agents
      // 预选兜底：项目无记录（或记录的 agent 已不在列）→ 首个可达 agent。
      if (!this.agent || !this.agents.some((a) => a.id === this.agent)) {
        this.agent = data.agents.find((a) => a.reachable)?.id ?? ''
      }
    } catch {
      // catalog 不可达：下拉如实降级（禁用 + 提示），绝不伪造选项。
      this.agents = []
      this.agent = ''
    }
  }

  private async loadCatalog(): Promise<void> {
    const { catalog, unavailable } = await loadModelCatalog()
    this.catalog = unavailable ? null : catalog
    this.catalogUnavailable = unavailable
    if (unavailable) {
      this.selectedProvider = null
      this.model = null
    } else {
      this.applyCatalogPreselect()
    }
  }

  /** 确认门禁：agent 必选（spec「dialog requires an explicit agent」）。 */
  private get confirmDisabled(): boolean {
    return !this.agent
  }

  private confirm(): void {
    if (this.confirmDisabled) return
    this.dispatchEvent(
      new CustomEvent<NewSessionDialogConfirm>('dialog-confirm', {
        detail: { agent: this.agent, model: this.model, mode: this.mode },
        bubbles: true,
        composed: true,
      }),
    )
  }

  private cancel(): void {
    this.dispatchEvent(
      new CustomEvent('dialog-cancel', { bubbles: true, composed: true }),
    )
  }

  render() {
    const providers = this.catalogProviders
    const providerModels = this.selectedProvider ? this.modelsFor(this.selectedProvider) : []
    return html`
      <wa-dialog
        label=${this.projectName ? `New session in ${this.projectName}` : 'New session'}
        style="--width: 460px;"
        ?open=${this.open}
        @wa-hide=${() => this.cancel()}
        data-testid="new-session-dialog"
      >
        <div class="form">
          <!-- agent 必选：唯一选 agent 的地方；不可达 agent 禁选并标注 cause。 -->
          <wa-select
            label="Agent"
            aria-label="Agent"
            data-testid="dialog-agent-select"
            value=${this.agent}
            ?disabled=${this.agents.length === 0}
            hoist
            @change=${(e: Event) => {
              this.agent = (e.target as HTMLSelectElement).value || ''
            }}
          >
            ${this.agents.length === 0
              ? html`<wa-option value="" disabled>agent catalog 不可用</wa-option>`
              : nothing}
            ${this.agents.map((a) =>
              a.id === 'native' && !a.reachable
                ? html`<wa-option value=${a.id} disabled
                    >${a.display} (unavailable: ${a.cause ?? 'unreachable'})</wa-option
                  >`
                : html`<wa-option value=${a.id} ?disabled=${!a.reachable}
                    >${a.reachable ? a.display : `${a.display} (unavailable: ${a.cause ?? ''})`}</wa-option
                  >`,
            )}
          </wa-select>

          <!-- 两级模型选择（可选）：目录不可得显式说明，绝不渲染空列表。 -->
          ${this.catalogUnavailable
            ? html`<p class="hint" data-testid="dialog-catalog-unavailable" role="status">
                模型目录不可用（Settings → Models 里配置 provider 后可选）
              </p>`
            : html`
                <wa-select
                  label="Provider"
                  aria-label="Provider"
                  data-testid="dialog-provider-select"
                  value=${this.selectedProvider ?? ''}
                  hoist
                  @change=${(e: Event) => {
                    const v = (e.target as HTMLSelectElement).value
                    this.selectedProvider = v
                    const models = this.modelsFor(v)
                    this.model = models[0] ?? null
                  }}
                >
                  ${providers.map((p) => html`<wa-option value=${p}>${p}</wa-option>`)}
                </wa-select>
                <wa-select
                  label="Model"
                  aria-label="Model"
                  data-testid="dialog-model-select"
                  value=${this.model ?? ''}
                  ?disabled=${providerModels.length === 0}
                  hoist
                  @change=${(e: Event) => {
                    this.model = (e.target as HTMLSelectElement).value || null
                  }}
                >
                  ${providerModels.map((m) => html`<wa-option value=${m}>${m}</wa-option>`)}
                </wa-select>
              `}

          <!-- 权限 mode（可选）：缺省「agent 默认」= wire 上省略 mode 字段。 -->
          <wa-select
            label="Permission mode"
            aria-label="Permission mode"
            data-testid="dialog-mode-select"
            value=${this.mode ?? ''}
            hoist
            @change=${(e: Event) => {
              // 缺省项（空值）= agent 默认：不发送 mode 字段。
              this.mode = (e.target as HTMLSelectElement).value || null
            }}
          >
            <wa-option value="">默认（逐次询问）</wa-option>
            <wa-option value="ask">ask（逐次询问）</wa-option>
            <wa-option value="edit">edit（自动接受编辑）</wa-option>
            <wa-option value="allow">allow（放行并留审计）</wa-option>
            <wa-option value="auto">auto（不门控，留审计）</wa-option>
          </wa-select>
          ${this.error
            ? html`<p class="error" data-testid="dialog-error" role="alert">${this.error}</p>`
            : nothing}
        </div>
        <wa-button
          slot="footer"
          variant="brand"
          data-testid="dialog-confirm"
          ?disabled=${this.confirmDisabled}
          @click=${() => this.confirm()}
          >创建会话</wa-button
        >
        <wa-button slot="footer" appearance="plain" data-testid="dialog-cancel" @click=${() => this.cancel()}
          >取消</wa-button
        >
      </wa-dialog>
    `
  }

  static styles = css`
    :host {
      display: block;
    }
    .form {
      display: flex;
      flex-direction: column;
      gap: var(--sebas-space-3);
      margin: 0;
    }
    .hint {
      margin: 0;
      font-size: 0.78rem;
      color: var(--sebas-text-faint);
      font-style: italic;
    }
    .error {
      margin: 0;
      font-size: 0.78rem;
      color: var(--sebas-status-failed);
    }
  `
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-new-session-dialog': SebasNewSessionDialog
  }
}
