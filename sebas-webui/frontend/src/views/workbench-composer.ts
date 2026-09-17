/**
 * Workbench composer (workbench-interaction-polish D2–D4：纯跟随模式)。
 *
 * The conversation input is SESSION-SCOPED and always in follow-up mode:
 * submit sends the message to the focused session
 * (`POST /api/sessions/{key}/message`); it never spawns a new one — creation
 * lives in the rail's creation dialog (`sebas-new-session-dialog`). The
 * bottom toolbar places the locked agent identity (immutable since creation)
 * at the LEFT and the model chip + submit control at the RIGHT. No settings
 * entry (the app shell owns settings), no permission-mode control (creation
 * mode choice lives in the dialog; mid-session switching stays in the
 * session header), no creation controls of any kind.
 *
 * Submit control state machine (design D4, priority order):
 *   POST 在途 (spinner) > turnInFlight && 无字 (红色停止方块 → cancel 链路)
 *   > turnInFlight && 有字 (排队形态，提交复用既有 turn-queue)
 *   > 有字 (send) > 禁用。
 *
 * Model chip (design D3): single chip at the bottom-right listing the
 * focused session's `available_models` in a two-level menu grouped by
 * provider — the grouping cross-references the shared Settings catalog
 * (`loadModelCatalog`); ids the catalog cannot place fall into an explicit
 * "会话提供" group at the bottom; a wholly unavailable catalog degrades to a
 * flat list. Switching goes through `session/set_config_option`
 * (`POST /api/sessions/{key}/model`). No models = explicit honest note.
 *
 * Core reachability is PUSHED (add-core-reachability-ws-push D4): the
 * composer consumes the structured state the app-shell passes down
 * (`coreReachability`, sourced from `core.reachability.get` + flip
 * notifications — no polling, no on-mount fetch). `ok=false` gates submit
 * (a submit would only bounce) with the reported cause; `null` (unknown) and
 * `ok=true` both leave submit available — the unknown default aligns with the
 * retired poller's pre-first-read behavior. A transient submit error is
 * surfaced inline via the shared `.callout-error` style and the message text
 * is preserved so the operator can retry.
 *
 * Command palette (session-slash-commands 3.1–3.2/D4): when the input's
 * first character is `/` and the focused session advertises commands
 * (`available_commands` — the agent's own advertisement, never hardcoded),
 * a filtered palette opens above the textarea (same floating recipe as the
 * model menu). Each row shows name + argument hint + description; ↑/↓ move
 * the highlight, Esc dismisses, and Enter/Tab are TWO-PHASE: the first
 * press completes the highlighted command (inserts `name + space`, keeps
 * focus for arguments) and never submits; after the arguments the next
 * Enter submits normally. Filtering is a live case-insensitive prefix match
 * on the token after `/`; no matches = no palette; a non-leading `/`
 * (`path/to`) never triggers it. Sessions without a command surface render
 * no palette and treat `/` as ordinary text (4.2 honest degradation).
 *
 * Interception (session-slash-commands 4.1/D3): with a command surface, a
 * `/`-prefixed submission whose command name is neither advertised nor the
 * universal built-in `compact` is blocked inline (agents like opencode
 * swallow unknown commands silently as empty turns) — the notice names the
 * command, nothing is sent, and editing the text clears it. The submission
 * itself is never rewritten (3.3): supported commands ride the ordinary
 * `api.sendMessage` path verbatim, busy/queued included (D5).
 */

import { LitElement, css, html, nothing, type PropertyValues } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type AvailableCommandInfo } from '../api/client.js'
import type { CoreReachabilityState } from '../api/ws.js'
import {
  loadModelCatalog,
  groupSessionModels,
  SESSION_PROVIDED_GROUP_LABEL,
  type ModelCatalog,
} from '../api/model-catalog.js'
import { icon } from '../components/icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/textarea/textarea.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'

/**
 * universal built-in 例外集（session-slash-commands D3）：claude 广告
 * `/compact`，opencode 硬编码处理但不广告——纯广告表会误拦它。钉为常量
 * 表，出现真实误伤时按证据扩充（spec「universal built-in」）。
 */
const UNIVERSAL_SLASH_COMMANDS: ReadonlySet<string> = new Set(['compact'])

/**
 * `/` 前缀输入的命令名解析（4.1）：文本以 `/` 开头时取第一个空白前的
 * token（不含 `/`）——`/goal`、`/goal clear`、`/compact` 都解析出命令名；
 * 裸 `/`（无 token）返回 null，按普通文本对待。
 */
function parseSlashCommandName(text: string): string | null {
  return /^\/(\S+)/.exec(text)?.[1] ?? null
}

/** 提交控件的五态（design D4 优先级渲染的判别值，测试按 data-state 断言）。 */
type SubmitState = 'disabled' | 'send' | 'sending' | 'stop' | 'queued'

/**
 * 一次性 composer 对焦请求的事件名（workbench-rail-polish 3.2/D2）：rail
 * 创建会话成功后派发、dashboard 接力到 `focusInput()`。沿 `sebas:*`
 * window 事件惯例（sebas:refetch / sebas:ws-state 同款）。仅创建成功这一个
 * 时机派发——会话切换（openSession/深链）绝不经过这里，不抢键盘焦点。
 */
export const COMPOSER_FOCUS_REQUEST = 'sebas:composer-focus'

@customElement('sebas-workbench-composer')
export class SebasWorkbenchComposer extends LitElement {
  /**
   * Focused session's encoded key. `null` = nothing focused: the composer
   * renders no creation controls and an explicit hint pointing at the rail's
   * creation entry (spec「no focus means the rail dialog」).
   */
  @property({ attribute: false }) sessionKey: string | null = null
  /**
   * Focused session's bound agent kind from the wire (`null` = the
   * configured default kind). Rendered read-only (🔒) at the bottom-left.
   */
  @property({ attribute: false }) agentKind: string | null = null
  /** Focused session's selectable models (agent configOptions). */
  @property({ attribute: false }) sessionModels: string[] = []
  /**
   * Focused session's agent-advertised slash commands (session-slash-commands
   * 2.2/3.1)。空表 = 无命令表面：不渲染面板、不拦截 `/` 输入（诚实退化）。
   */
  @property({ attribute: false }) sessionCommands: AvailableCommandInfo[] = []
  /** Focused session's current model id. */
  @property({ attribute: false }) currentModel: string | null = null
  /**
   * （workbench-interaction-polish 4.3，design D4）聚焦会话是否有 turn 在飞
   * （fix-pending-queue-liveness 3.1 起：dashboard 供数 = 引擎事实
   * `turn_engaged`（WORKING ∨ 泊车 ∨ spawn 窗口），旧 core 缺省回退
   * status_slug === 'working'）。turn 结束（WS 推送）自动复位。
   */
  @property({ type: Boolean }) turnInFlight = false
  /**
   * （fix-pending-queue-liveness 3.2）聚焦会话在等操作者的权限批复（泊车）。
   * turn 在飞 + 泊车：排队形态附「等待你的审批」指示（提交读作排在一次可
   * 回答的提问后面）；空输入仍是停止方块（spec「stop stays reachable while
   * parked」）。
   */
  @property({ type: Boolean }) waitingApproval = false
  /**
   * （workbench-live-conversation-flow 3.2）聚焦会话的子进程正在拉起
   * （0-turn 占位激活中 / Dormant resume 在途）。模型芯片据此显示
   * 「启动中…」而不是误导性的「无可用模型」——后者只在 spawn 完成
   * 且 agent 确实没报模型时出现。
   */
  @property({ type: Boolean }) childStarting = false
  /**
   * （4.1）mode 切换自会话头迁入底沿左端：当前期望 mode（desired，
   * 远端会话为节点回报值）。`null` = 会话未记录 mode（创建表单的
   * 「agent 默认」）。
   */
  @property({ attribute: false }) currentMode: string | null = null
  /** mode 切换可用性：0-turn 占位（无 session_id）不可切——mode 由创建表单决定。 */
  @property({ type: Boolean }) modeEditable = false
  /**
   * （add-core-reachability-ws-push D4）shell 下传的结构化核心可达性（
   * `core.reachability.get` 初始化 + 翻转通知更新，dashboard 纯透传）。
   * `null` = 未知：对齐旧轮询器的首个读数前行为，不禁用提交门。
   */
  @property({ attribute: false }) coreReachability: CoreReachabilityState | null = null

  @state() private text = ''
  @state() private sending = false
  @state() private error: string | null = null
  /**
   * Settings 目录（design D3 分组交叉引用的数据源）：只用于把会话平铺模型
   * id 归回 provider 组；目录不可得时芯片退化为平铺，不伪造分组。
   */
  @state() private catalog: ModelCatalog | null = null
  /** 中程切换聚焦会话模型时的在途标记（add-acp-model-selection 语义）。 */
  @state() private modelSwitching = false
  /** 中程切换会话权限模式的在途标记（add-agent-mode-selection 语义）。 */
  @state() private modeSwitching = false
  /** 模型芯片两级菜单的开合（design D3）。 */
  @state() private modelMenuOpen = false
  /**
   * 命令面板高亮项下标（session-slash-commands 3.1）：面板打开即高亮首位，
   * ↑/↓ 移动；文本变化时回到首位。
   */
  @state() private paletteIndex = 0
  /** Esc 关闭后的面板作废标记；文本一变即重新获得开启资格。 */
  @state() private paletteDismissed = false
  /**
   * 4.1 拦截提示（「该会话的 agent 不支持此命令：/xxx」）：就地呈现、不发
   * 请求；用户修改文本后清除，可再提交。
   */
  @state() private slashNotice: string | null = null

  /**
   * 提交门的不可达视图（add-core-reachability-ws-push D4）：消费下传的
   * `coreReachability`，ok=false 时携带 cause；未知（null）与可达均为
   * null——不禁用。判定单一出处，输入门/提交状态机/横幅共用。
   */
  private get unreachable(): { cause: string } | null {
    const r = this.coreReachability
    if (r !== null && r.ok === false) {
      return { cause: r.cause ?? 'core not connected' }
    }
    return null
  }

  private reloadCatalogBound = (): void => {
    void this.loadCatalog()
  }

  connectedCallback(): void {
    super.connectedCallback()
    void this.loadCatalog()
    // defaults/catalog 变更（管理页 set/clear）即时反映到芯片分组。
    window.addEventListener('sebas:refetch', this.reloadCatalogBound)
  }

  disconnectedCallback(): void {
    window.removeEventListener('sebas:refetch', this.reloadCatalogBound)
    this.removeMenuDismissListeners()
    super.disconnectedCallback()
  }

  protected updated(changed: PropertyValues): void {
    // 聚焦会话变了（切换/关闭/新建跳转）：收起模型菜单、清掉上一个会话的
    // 输入残留交给调用方……文本保留是既有语义（失败重试），这里只在会话
    // 真正更换时清空，避免把 A 会话的草稿发进 B 会话。
    if (changed.has('sessionKey')) {
      this.modelMenuOpen = false
      // 命令面板与拦截提示随旧会话作废（新会话的命令表由 dashboard 重取
      // detail 后经 sessionCommands 到达）。
      this.paletteDismissed = false
      this.paletteIndex = 0
      this.slashNotice = null
      const prev = changed.get('sessionKey')
      if (prev !== undefined && this.sessionKey !== prev) this.text = ''
    }
    // 文本变化（打字/补全/提交清空）：面板重新获得开启资格、高亮回首位，
    // 撤销上一次的 Esc 作废；拦截提示就地清除（4.1「改字后可再提交」）。
    if (changed.has('text')) {
      this.paletteDismissed = false
      this.paletteIndex = 0
      this.slashNotice = null
    }
  }

  /** 目录加载（design D2：与创建对话框共用 loadModelCatalog，防漂移）。 */
  private async loadCatalog(): Promise<void> {
    const { catalog } = await loadModelCatalog()
    this.catalog = catalog
  }

  /** 跟随模式输入门禁：无聚焦会话或 core 不可达 = 禁用。 */
  private inputDisabled(): boolean {
    return this.sending || this.sessionKey === null || this.unreachable !== null
  }

  /**
   * 提交控件状态机（design D4，优先级从高到低）：POST 在途（转圈）>
   * turn 在飞且无字（停止方块）> turn 在飞且有字（排队形态）> 有字
   * （send）> 禁用。
   */
  private submitState(): SubmitState {
    if (this.sending) return 'sending'
    if (this.sessionKey === null || this.unreachable !== null) return 'disabled'
    const hasText = this.text.trim().length > 0
    if (this.turnInFlight) return hasText ? 'queued' : 'stop'
    return hasText ? 'send' : 'disabled'
  }

  /**
   * 把键盘焦点落进输入框（workbench-rail-polish 3.2/D2，公开方法）：创建
   * 会话成功后由 dashboard 代为调用一次。先等本组件把当前状态渲染完——
   * 聚焦 key 刚从 summary 到达时 wa-textarea 可能尚未上屏，直接查会扑空；
   * wa-textarea 自带 focus 转发到内部原生 textarea，键盘焦点真正落在
   * 输入框里。仅创建流程走这里；openSession 等切换路径绝不调用。
   */
  async focusInput(): Promise<void> {
    await this.updateComplete
    this.renderRoot.querySelector('wa-textarea')?.focus()
  }

  private async submit(): Promise<void> {
    const key = this.sessionKey
    if (!key) return
    const prompt = this.text.trim()
    if (!prompt) return
    if (this.sending || this.unreachable !== null) return
    // 拦截门（session-slash-commands 4.1/D3）：有命令表面的会话，未广告且
    // 非 universal built-in 的命令就地阻止——opencode 会把未知命令静默吞成
    // 空回合，发出去只能是空转。文案点名命令、保留输入；改字后可再提交。
    const unsupported = this.interceptUnsupported(prompt)
    if (unsupported !== null) {
      this.slashNotice = `该会话的 agent 不支持此命令：/${unsupported}`
      return
    }
    this.sending = true
    this.error = null
    try {
      // turn 在飞时服务端自动排队（workbench-turn-queue）——composer 不区分
      // 开轮与排队，提交语义一条路径。
      await api.sendMessage(key, prompt)
      this.text = ''
      // 舞台就地刷新：dashboard 监听后立刻重取聚焦 detail（WS 推送之外的
      // 乐观刷新，避免等下一个 summary 周期）。
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.sending = false
    }
  }

  /**
   * 停止（design D5）：点击红色方块 → 新 cancel 链路
   * （POST /api/sessions/{key}/cancel）。错误走既有 callout；turn 结束
   * （WS 推送 turnInFlight=false）自动回到 send 态。
   */
  private async cancelTurn(): Promise<void> {
    const key = this.sessionKey
    if (!key) return
    this.error = null
    try {
      await api.cancelSession(key)
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    }
  }

  /** 跟随模式的模型切换：`session/set_config_option`（add-acp-model-selection）。 */
  private async switchModel(modelId: string): Promise<void> {
    const key = this.sessionKey
    if (!key || this.modelSwitching) return
    this.modelSwitching = true
    this.error = null
    try {
      await api.setSessionModel(key, modelId)
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.modelSwitching = false
    }
  }

  /** 底沿左端的 mode 切换（add-agent-mode-selection 通道不变）。 */
  private async switchMode(mode: string): Promise<void> {
    const key = this.sessionKey
    if (!key || this.modeSwitching) return
    this.modeSwitching = true
    this.error = null
    try {
      await api.setSessionMode(key, mode)
      this.dispatchEvent(
        new CustomEvent('composer-sent', { detail: { key }, bubbles: true, composed: true }),
      )
    } catch (e) {
      this.error = String(e)
    } finally {
      this.modeSwitching = false
    }
  }

  /**
  // ── 命令面板（session-slash-commands 3.1–3.3 / 4.1–4.2，design D3/D4/D5）──

  /**
   * 面板触发条件：文本以 `/` 开头且尚无空白——用户仍在命令名段（`/`、
   * `/go`）。参数段（`/goal clear`）与非首位 `/`（`path/to`）都不触发；
   * 返回的值是 `/` 之后的实时过滤前缀（小写化，3.2 不区分大小写）。
   */
  private palettePrefix(): string | null {
    if (!this.text.startsWith('/') || /\s/.test(this.text)) return null
    return this.text.slice(1).toLowerCase()
  }

  /** 前缀匹配后的候选（3.2）：无会话表面 → 空表（无面板，诚实退化 4.2）。 */
  private filteredCommands(): AvailableCommandInfo[] {
    const prefix = this.palettePrefix()
    if (prefix === null || this.sessionCommands.length === 0) return []
    return this.sessionCommands.filter((c) => c.name.toLowerCase().startsWith(prefix))
  }

  /** 面板开合的单一判据：有候选且未被 Esc 作废。无候选 = 不渲染（4.2 空态不显示）。 */
  private paletteOpen(): boolean {
    return !this.paletteDismissed && this.filteredCommands().length > 0
  }

  private movePaletteHighlight(delta: number): void {
    const count = this.filteredCommands().length
    if (count === 0) return
    this.paletteIndex = Math.min(count - 1, Math.max(0, this.paletteIndex + delta))
  }

  /**
   * 两段式第一段（3.1）：把 `name + 空格` 插入输入框、焦点留在输入框补
   * 参数、面板关闭（文本含空白后 palettePrefix() 失效）。参数补完后的
   * 下一次 Enter 走普通提交。
   */
  private completeCommand(name: string): void {
    this.text = `/${name} `
    const ta = this.shadowRoot?.querySelector('wa-textarea')
    ta?.focus()
  }

  /**
   * 输入区键盘路由（design D4）：面板打开期间 ↑/↓ 移动高亮、Esc 关闭、
   * Enter/Tab 一律两段式补全——绝不发送（两段式语义：第一段永远只是把
   * 命令补进输入框）。面板未开时维持既有语义：普通 Enter 发送、
   * Shift+Enter 换行、IME 组词回车不触发。
   */
  private onInputKeydown(e: KeyboardEvent): void {
    if (this.paletteOpen()) {
      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          this.movePaletteHighlight(1)
          return
        case 'ArrowUp':
          e.preventDefault()
          this.movePaletteHighlight(-1)
          return
        case 'Escape':
          e.stopPropagation()
          this.paletteDismissed = true
          return
        case 'Enter':
        case 'Tab':
          if (!e.shiftKey && !e.isComposing) {
            e.preventDefault()
            // 高亮钳位到当前候选范围内再补全（防文本变化与高亮复位之间
            // 的瞬态越界把 Enter 吃成空操作）。
            const items = this.filteredCommands()
            const idx = Math.min(Math.max(this.paletteIndex, 0), items.length - 1)
            const item = items[idx]
            if (item) this.completeCommand(item.name)
          }
          return
      }
      // 其余按键（继续打字等）不拦截：面板随文本实时重过滤。
      return
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault()
      void this.submit()
    }
  }

  /**
   * 4.1/D3 拦截判定：返回应拦截的命令名（null = 放行）。规则——会话无
   * 命令表面（空表，native 等）不拦截；非 `/` 前缀不拦截；命令名 ∈ 广告表
   * ∨ ∈ universal built-in（compact）放行；其余拦截。
   */
  private interceptUnsupported(prompt: string): string | null {
    if (this.sessionCommands.length === 0) return null
    if (!prompt.startsWith('/')) return null
    const name = parseSlashCommandName(prompt)
    if (name === null) return null
    if (this.sessionCommands.some((c) => c.name === name)) return null
    if (UNIVERSAL_SLASH_COMMANDS.has(name)) return null
    return name
  }

  /** 面板浮层（3.1）：定位与视觉照抄 model 菜单（textarea 上方弹出）。 */
  private renderCommandPalette() {
    if (!this.paletteOpen()) return nothing
    const items = this.filteredCommands()
    return html`
      <div
        class="cmd-palette"
        role="listbox"
        aria-label="Session commands"
        data-testid="command-palette"
      >
        ${items.map((c, i) => {
          const selected = i === this.paletteIndex
          return html`
            <button
              class="menu-item ${selected ? 'highlighted' : ''}"
              type="button"
              role="option"
              aria-selected=${selected ? 'true' : 'false'}
              data-command=${c.name}
              @click=${() => this.completeCommand(c.name)}
            >
              <span class="menu-item-label">/${c.name}</span>
              ${c.hint ? html`<span class="cmd-hint">${c.hint}</span>` : nothing}
              ${c.description
                ? html`<span class="cmd-desc">${c.description}</span>`
                : nothing}
            </button>
          `
        })}
      </div>
    `
  }

  // ─── 模型芯片（design D3）────────────────────────────────────────────

  private openModelMenu(): void {
    if (this.sessionModels.length === 0) return
    this.modelMenuOpen = true
    // 捕获阶段监听 document 点击：点菜单外任意处收起（打开菜单的那次点击
    // 已过捕获阶段，不会立刻自吞）。
    document.addEventListener('click', this.dismissMenuOnDocClick, true)
  }

  private closeModelMenu(): void {
    this.modelMenuOpen = false
    this.removeMenuDismissListeners()
  }

  private dismissMenuOnDocClick = (e: Event): void => {
    const path = e.composedPath()
    if (!path.includes(this)) this.closeModelMenu()
  }

  private menuKeydown = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.stopPropagation()
      this.closeModelMenu()
    }
  }

  private removeMenuDismissListeners(): void {
    document.removeEventListener('click', this.dismissMenuOnDocClick, true)
  }

  /**
   * 分组视图（design D3）：目录交叉引用 → provider 组；查不到 → 「会话提供」
   * 置底；目录不可得 → 单组平铺（消费方据 groups.length===1 &&
   * provider===null 识别平铺态，不渲染组头）。
   */
  private modelGroups() {
    return groupSessionModels(this.sessionModels, this.catalog)
  }

  private renderModelChip() {
    if (this.sessionModels.length === 0) {
      // 子进程拉起中（workbench-live-conversation-flow 3.2）：模型表要等
      // agent 上报，此刻「无可用模型」是误导——显式呈现启动中。
      if (this.childStarting) {
        return html`<span
          class="label placeholder model-chip-empty"
          data-testid="model-chip-starting"
          role="status"
          title="子进程拉起中，模型表随后可用"
          >启动中…</span
        >`
      }
      // 显式诚实态：该会话无可选模型，绝不渲染空菜单（agent-workbench
      // delta「chip without session models is stated honestly」）。
      return html`<span
        class="label placeholder model-chip-empty"
        data-testid="model-chip-unavailable"
        role="status"
        title="该会话未提供可选模型"
        >无可用模型</span
      >`
    }
    const groups = this.modelGroups()
    const flat = groups.length === 1 && groups[0]!.provider === null
    const label = this.currentModel ?? this.sessionModels[0]!
    return html`
      <div class="model-wrap">
        <button
          class="chip"
          type="button"
          data-testid="model-chip"
          aria-haspopup="listbox"
          aria-expanded=${this.modelMenuOpen ? 'true' : 'false'}
          title="会话模型（切换走 session/set_config_option）"
          @click=${() =>
            this.modelMenuOpen ? this.closeModelMenu() : this.openModelMenu()}
        >
          ${icon('zap', 12)}<span class="chip-label">${label}</span>
        </button>
        ${this.modelMenuOpen
          ? html`
              <div
                class="model-menu"
                role="listbox"
                aria-label="Session model"
                data-testid="model-menu"
                @keydown=${this.menuKeydown}
              >
                ${groups.map((g) =>
                  flat
                    ? this.renderModelItems(g.models)
                    : html`
                        <div class="menu-group">
                          <div class="menu-group-label" data-testid="model-group">
                            ${g.provider ?? SESSION_PROVIDED_GROUP_LABEL}
                          </div>
                          ${this.renderModelItems(g.models)}
                        </div>
                      `,
                )}
              </div>
            `
          : nothing}
      </div>
    `
  }

  private renderModelItems(models: string[]) {
    return models.map(
      (m) => html`
        <button
          class="menu-item"
          type="button"
          role="option"
          aria-selected=${this.currentModel === m ? 'true' : 'false'}
          data-model=${m}
          ?disabled=${this.modelSwitching}
          @click=${() => {
            this.closeModelMenu()
            if (m !== this.currentModel) void this.switchModel(m)
          }}
        >
          <span class="menu-item-label">${m}</span>
          ${this.currentModel === m ? html`<span class="check" aria-hidden="true">✓</span>` : nothing}
        </button>
      `,
    )
  }

  /**
   * 泊车指示（fix-pending-queue-liveness 3.2）：会话在等操作者的权限批复时
   * 贴着提交控件就地呈现——排队提交读作「排在一次可回答的提问后面」，而
   * 不是消失进一个无法解释的队列（spec「submission while a permission
   * prompt is parked queues visibly」）。空输入的停止态同样可见（停止在
   * 泊车态可达的另一半契约）。
   */
  private renderParkedHint() {
    if (!this.turnInFlight || !this.waitingApproval) return nothing
    return html`<span
      class="parked-hint"
      data-testid="parked-hint"
      role="status"
      title="会话在等你的权限批复：提交将排队，直到你处理审批"
      >等待你的审批</span
    >`
  }

  /** 提交控件：按 submitState() 渲染五态（design D4）。 */
  private renderSubmitButton() {
    const state = this.submitState()
    const meta: Record<
      SubmitState,
      { label: string; icon: ReturnType<typeof icon>; disabled: boolean }
    > = {
      disabled: { label: 'Send', icon: icon('forward', 14), disabled: true },
      send: { label: 'Send', icon: icon('forward', 14), disabled: false },
      sending: {
        label: '发送中',
        icon: html`<span class="spinner" aria-hidden="true"></span>`,
        disabled: true,
      },
      stop: { label: '停止回复', icon: icon('stop', 14), disabled: false },
      queued: { label: '排队提交', icon: icon('clock', 14), disabled: false },
    }
    const m = meta[state]
    const onClick =
      state === 'stop'
        ? () => void this.cancelTurn()
        : state === 'send' || state === 'queued'
          ? () => void this.submit()
          : () => {}
    return html`
      <button
        class="send-button ${state}"
        type="button"
        data-state=${state}
        data-testid="submit-control"
        aria-label=${m.label}
        title=${m.label}
        ?disabled=${m.disabled}
        @click=${onClick}
      >
        ${m.icon}
      </button>
    `
  }

  render() {
    // 无聚焦会话：空态提示（dashboard 的 empty-stream 承接主提示，composer
    // 就地给一条指向 rail 创建入口的显式说明），不渲染任何创建控件。
    if (this.sessionKey === null) {
      return html`
        ${this.renderBanners()}
        <div class="composer no-focus" data-testid="composer-no-focus">
          <span class="no-focus-hint">
            在左侧项目栏的 <b>+</b> 新建会话后，这里开始对话。
          </span>
        </div>
      `
    }
    return html`
      ${this.renderBanners()}
      <div class="composer">
        <div class="input-wrap">
          ${this.renderCommandPalette()}
          <wa-textarea
            placeholder="Ask for follow-up changes…"
            aria-label="Message"
            resize="none"
            ?disabled=${this.inputDisabled()}
            .value=${this.text}
            @input=${(e: Event) => (this.text = (e.target as HTMLTextAreaElement).value)}
            @keydown=${(e: KeyboardEvent) => this.onInputKeydown(e)}
          ></wa-textarea>
        </div>
        <div class="composer-bottom">
          <div class="left-tools">
            ${this.modeEditable
              ? html`<wa-select
                  class="mode-select"
                  size="xs"
                  hoist
                  value=${this.currentMode ?? ''}
                  ?disabled=${this.modeSwitching}
                  aria-label="Session mode"
                  data-testid="mode-switch"
                  @change=${(e: Event) => {
                    const v = (e as unknown as { target: { value: string } }).target.value
                    if (v) void this.switchMode(v)
                  }}
                >
                  <wa-option value="ask">ask</wa-option>
                  <wa-option value="edit">edit</wa-option>
                  <wa-option value="allow">allow</wa-option>
                  <wa-option value="auto">auto</wa-option>
                </wa-select>`
              : nothing}
          </div>
          <div class="right-tools">
            ${this.renderModelChip()}
            ${this.renderParkedHint()}
            ${this.renderSubmitButton()}
          </div>
        </div>
      </div>
    `
  }

  private renderBanners() {
    return html`
      ${this.unreachable
        ? html`
            <div class="callout callout-warning" role="status">
              ${icon('alert')}<span>core not connected: ${this.unreachable.cause}</span>
            </div>
          `
        : nothing}
      ${this.error
        ? html`
            <div class="callout callout-error" role="alert" data-testid="composer-error">
              ${icon('alert')}<span>${this.error}</span>
            </div>
          `
        : nothing}
      ${this.slashNotice
        ? html`
            <div class="callout callout-warning" role="status" data-testid="slash-unsupported">
              ${icon('alert')}<span>${this.slashNotice}</span>
            </div>
          `
        : nothing}
    `
  }

  static styles = [
    viewStyles,
    css`
      :host {
        /* 宿主吃满 dashboard 分配的输入框分割面（5.2：拖出的高度给输入
           区），无聚焦提示态同理。 */
        display: flex;
        flex-direction: column;
        min-height: 0;
      }
      /* Composer: ONE rounded shell (浮岛视觉 D6 与预览原型同款), the
       * wa-textarea inside is chrome-stripped so the shell is the only
       * visible card. */
      .composer {
        flex: 1;
        min-height: 0;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: 18px;
        padding: var(--sebas-space-3);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
      }
      .composer wa-textarea {
        width: 100%;
        flex: 1;
        min-height: 36px;
      }
      /* 输入区容器：命令面板的定位锚（design D4 照抄 model-wrap 的浮层
         姿势）——面板贴 textarea 上方弹出。 */
      .input-wrap {
        position: relative;
        display: flex;
        flex-direction: column;
        flex: 1;
        min-height: 0;
      }
      /* 命令面板（session-slash-commands 3.1）：贴 textarea 上方，配方同
         .model-menu（surface 底 + strong 边 + shadow-2，页底组件一律向上弹）。 */
      .cmd-palette {
        position: absolute;
        left: 0;
        right: 0;
        bottom: calc(100% + 6px);
        max-height: 260px;
        overflow-y: auto;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border-strong);
        border-radius: var(--sebas-radius-md);
        box-shadow: var(--sebas-shadow-2);
        padding: 4px;
        z-index: 20;
      }
      .cmd-palette .menu-item {
        flex-wrap: wrap;
        row-gap: 2px;
      }
      /* 键盘高亮态：与 hover 同视觉（design D4 键盘可达）。 */
      .cmd-palette .menu-item.highlighted {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .cmd-palette .cmd-hint {
        flex: 0 1 auto;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        color: var(--sebas-text-faint);
        font-style: italic;
      }
      .cmd-palette .cmd-desc {
        flex-basis: 100%;
        font-size: 0.7rem;
        color: var(--sebas-text-faint);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      /* 剥掉 wa-textarea 自带的底色/边框/阴影，只留纯文本输入区；高度随
         分割面拉伸（resize=none 关掉组件自带的拖角，5.2 的分割线是唯一的
         高度控制面），内容多时输入区内部滚动。 */
      .composer wa-textarea::part(base) {
        background: transparent;
        border: none;
        box-shadow: none;
        height: 100%;
        min-height: 36px;
        padding: 4px 8px;
        overflow-y: auto;
      }
      /* 8.1: the native textarea lives in wa-textarea's shadow root, so the
         shared focus-visible rule can't reach it — ring the host instead.
         5.5 / design D5: composer focus is "your move", so the ring is the
         --signal accent, not the shared indigo focus ring. */
      .composer wa-textarea:focus-within {
        outline: 2px solid var(--sebas-signal);
        outline-offset: 2px;
        border-radius: var(--sebas-radius-sm);
      }
      /* 无聚焦会话的显式提示（spec：指向 rail 创建入口，绝不就地给创建控件）。 */
      .composer.no-focus {
        align-items: center;
        justify-content: center;
        padding: var(--sebas-space-4);
      }
      .no-focus-hint {
        font-size: 0.82rem;
        color: var(--sebas-text-dim);
      }
      .no-focus-hint b {
        color: var(--sebas-text-bright);
      }
      /* Bottom toolbar: locked agent identity LEFT, model chip + submit
         RIGHT（agent-workbench delta「toolbar composition」）。 */
      .composer-bottom {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
      }
      .composer-bottom .left-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        min-width: 0;
      }
      .composer-bottom .right-tools {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-2);
        margin-left: auto;
      }
      .composer-bottom .label {
        font-family: var(--sebas-font-mono);
      }
      .composer-bottom .label.placeholder {
        color: var(--sebas-text-faint);
        letter-spacing: 0.15em;
      }
      /* ── 模型芯片（design D3）────────────────────────────────────────── */
      .model-wrap {
        position: relative;
        display: inline-flex;
        min-width: 0;
      }
      .chip {
        display: inline-flex;
        align-items: center;
        gap: 6px;
        max-width: 260px;
        border: 1px solid var(--sebas-border);
        background: var(--sebas-surface-2);
        border-radius: 999px;
        padding: 2px 10px;
        font: inherit;
        font-size: 0.72rem;
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-dim);
        cursor: pointer;
        transition:
          color var(--sebas-dur) var(--sebas-ease),
          border-color var(--sebas-dur) var(--sebas-ease);
      }
      .chip:hover {
        color: var(--sebas-text-bright);
        border-color: var(--sebas-border-strong);
      }
      .chip:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      .chip .chip-label {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .model-chip-empty {
        font-size: 0.72rem;
        letter-spacing: 0.05em;
        font-style: italic;
      }
      /* 两级分组菜单：贴芯片上方弹出（composer 在页底）。 */
      .model-menu {
        position: absolute;
        right: 0;
        bottom: calc(100% + 6px);
        min-width: 240px;
        max-height: 280px;
        overflow-y: auto;
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border-strong);
        border-radius: var(--sebas-radius-md);
        box-shadow: var(--sebas-shadow-2);
        padding: 4px;
        z-index: 20;
      }
      .menu-group + .menu-group {
        margin-top: 4px;
      }
      .menu-group-label {
        font-size: 0.66rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.08em;
        color: var(--sebas-text-faint);
        padding: 4px 8px 2px;
      }
      .menu-item {
        display: flex;
        align-items: center;
        gap: 8px;
        width: 100%;
        border: none;
        background: none;
        border-radius: var(--sebas-radius-sm);
        padding: 5px 8px;
        font: inherit;
        font-size: 0.76rem;
        font-family: var(--sebas-font-mono);
        color: var(--sebas-text-dim);
        cursor: pointer;
        text-align: left;
        transition:
          background var(--sebas-dur) var(--sebas-ease),
          color var(--sebas-dur) var(--sebas-ease);
      }
      .menu-item:hover:enabled {
        background: var(--sebas-surface-2);
        color: var(--sebas-text-bright);
      }
      .menu-item:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: -1px;
      }
      .menu-item[aria-selected='true'] {
        color: var(--sebas-accent);
      }
      .menu-item .menu-item-label {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .menu-item .check {
        flex: 0 0 auto;
        color: var(--sebas-accent);
        font-weight: 700;
      }
      /* ── 提交控件状态机（design D4）──────────────────────────────────── */
      /* 泊车指示（fix-pending-queue-liveness 3.2）：提交控件左侧的就地状态
         词——等待批复 = 你的回合。signal 强调色与审查卡同源。 */
      .parked-hint {
        font-size: 0.72rem;
        color: var(--sebas-signal);
        white-space: nowrap;
      }
      /* 28px accent icon send button; disabled dims instead of vanishing. */
      .send-button {
        width: 28px;
        height: 28px;
        display: grid;
        place-items: center;
        background: var(--sebas-accent);
        color: var(--sebas-accent-ink);
        border: none;
        border-radius: var(--sebas-radius-md);
        cursor: pointer;
        padding: 0;
        transition:
          opacity var(--sebas-dur) var(--sebas-ease),
          background var(--sebas-dur) var(--sebas-ease);
      }
      .send-button:disabled {
        opacity: 0.35;
        cursor: not-allowed;
      }
      .send-button:hover:enabled {
        filter: brightness(1.05);
      }
      .send-button:focus-visible {
        outline: var(--sebas-focus-ring);
        outline-offset: 2px;
      }
      /* 流式且输入为空：红色停止方块（点击走 cancel 链路）。 */
      .send-button.stop {
        background: var(--sebas-status-failed);
        opacity: 1;
      }
      /* 流式且有字：排队形态（queued 色，提交进既有 turn-queue）。 */
      .send-button.queued {
        background: var(--sebas-status-queued);
        color: #062a2e;
        opacity: 1;
      }
      /* POST 在途：转圈。 */
      .send-button .spinner {
        width: 13px;
        height: 13px;
        border-radius: 50%;
        border: 2px solid var(--sebas-accent-ink);
        border-top-color: transparent;
        animation: sebas-spin 0.8s linear infinite;
      }
      @keyframes sebas-spin {
        to {
          transform: rotate(360deg);
        }
      }
      @media (prefers-reduced-motion: reduce) {
        /* reduced-motion：转圈降为静态指示（全局 tokens 的 reduced-motion
           规则不进 shadow DOM，组件内自行遵守既有约定）。 */
        .send-button .spinner {
          animation: none;
        }
      }
    `,
  ]
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-workbench-composer': SebasWorkbenchComposer
  }
}
