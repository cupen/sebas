/**
 * 待生效堆叠区（workbench-turn-queue 7.1–7.3；fix-pending-queue-liveness
 * 修订 D8 为两级反馈）。
 *
 * 渲染在 composer 上方、投递序列出聚焦会话的待生效提交（pending
 * submissions），不占 transcript 空间。每个条目声明处置：
 *   - staging → 「将并入首条消息」（spawn 窗口暂存，激活时合并）；
 *   - turn → 「待执行 · 第 N 位」（N 为组内 1 基位置）。
 *
 * 队列不前进时（fix-pending-queue-liveness 3.2）条目注明原因：按
 * `turnEngaged`（回合占用）与 `waitingApproval`（泊车审批在等）渲染
 * 「等待你的审批」/「等待当前回合结束」+ 起等时刻——空转会话 + 增长的栈
 * 永不无解释。起等时刻是组件侧锚点（首次观察到阻塞条件的时刻），随条件
 * 解除/会话切换重置。
 *
 * 交互面：
 *   - 逐条删除（× 按钮）；
 *   - HTML5 拖拽重排（仅同处置组内；优先项 /btw 不可拖、不可被越过——
 *     「先判后动」：非法落点不发请求、不做乐观更新）；
 *   - 键盘可达的上移/下移按钮（纯拖拽不可达，仓库有 a11y 门禁）。
 *
 * 拒绝反馈两级判据（fix-pending-queue-liveness 3.3，design D4）：操作后以
 * 服务端 post-op 真相为判——条目已不在（被并发开始消费）= 竞态竞输，静默
 * 对账；条目仍在 / 4xx / 网络失败 = 确定性拒绝，走分级通知低档（warn）
 * 就地点名条目与原因。堆叠区对服务端未执行的操作绝不无感。
 *
 * 会话终结（7.3）：`dropped` 非空时渲染一次性「未执行」提示，逐条点名
 * 被丢弃的提交——堆叠区随会话消失，提示是唯一记录。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { ApiError, NetworkError, api, type PendingSubmission } from '../api/client.js'
import { notify } from '../notify.js'

@customElement('sebas-pending-stack')
export class SebasPendingStack extends LitElement {
  /** 聚焦会话的待生效提交（投递序，来自会话 payload）。 */
  @property({ attribute: false }) pending: PendingSubmission[] = []
  /** 会话的 encoded key；null = 无聚焦会话（堆叠区隐藏）。 */
  @property({ attribute: false }) sessionKey: string | null = null
  /** （7.3）会话终结时被丢弃的提交；null = 无提示。非空数组渲染一次性提示。 */
  @property({ attribute: false }) dropped: PendingSubmission[] | null = null
  /**
   * （fix-pending-queue-liveness 3.2）聚焦会话的回合是否被占用（引擎事实
   * `turn_engaged`：WORKING ∨ 泊车 ∨ spawn 窗口）。true 且栈内有 turn 条目
   * = 队列没有前进，条目注明原因与起等时刻。
   */
  @property({ type: Boolean }) turnEngaged = false
  /**
   * （fix-pending-queue-liveness 3.2）回合被占用是因为在等操作者的权限批复
   * （泊车）。阻塞原因随之呈现为「等待你的审批」而不是「等待当前回合结束」。
   */
  @property({ type: Boolean }) waitingApproval = false

  /** 乐观视图：操作在途时覆盖展示，成功即被服务端返回值清空。 */
  @state() private optimistic: PendingSubmission[] | null = null
  /** 操作在途的条目 id（禁用其再次拖拽/操作）。 */
  @state() private busyId: number | null = null
  /** HTML5 拖拽进行中的条目 id。 */
  @state() private dragId: number | null = null
  /**
   * 阻塞条件的起等锚点（Date.now() 毫秒）：首次观察到「回合占用且栈内有
   * turn 条目」的时刻；条件解除 / 会话切换即重置。纯呈现侧状态——刷新后
   * 重新起算（不伪造一个服务端时刻）。
   */
  @state() blockedSince: number | null = null

  protected willUpdate(changed: Map<string, unknown>): void {
    // 服务端对账后的下一次全量刷新（prop 更新）接管真相，撤销乐观覆盖。
    if (changed.has('pending')) this.optimistic = null
    // 会话切换：上一个会话的阻塞锚点作废。
    if (changed.has('sessionKey')) this.blockedSince = null
    // 阻塞条件的进入/离开：进入记锚点，离开清锚点。
    const blocked =
      this.turnEngaged && this.view.some((x) => x.disposition === 'turn')
    if (blocked && this.blockedSince === null) this.blockedSince = Date.now()
    if (!blocked && this.blockedSince !== null) this.blockedSince = null
  }

  static styles = css`
    :host { display: block; }
    .stack {
      display: flex;
      flex-direction: column;
      gap: 4px;
      padding: 6px 8px;
      margin-bottom: 6px;
      background: var(--sebas-surface);
      border: 1px solid var(--sebas-border);
      border-radius: 12px;
    }
    .stack-title {
      font-size: 0.66rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: var(--sebas-text-faint);
    }
    .entry {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 4px 6px;
      border-radius: var(--sebas-radius-sm);
      font-size: 0.78rem;
      color: var(--sebas-text-dim);
      background: var(--sebas-surface-2);
      border: 1px solid transparent;
    }
    .entry.draggable { cursor: grab; }
    .entry.drag-over { box-shadow: inset 0 2px 0 var(--sebas-accent); }
    .entry.dragging { opacity: 0.4; }
    .entry.busy { opacity: 0.6; }
    .text {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .disp {
      flex-shrink: 0;
      font-size: 0.68rem;
      color: var(--sebas-text-faint);
      font-variant-numeric: tabular-nums;
    }
    .prio {
      flex-shrink: 0;
      font-size: 0.66rem;
      color: var(--sebas-accent);
      background: var(--sebas-accent-soft);
      border-radius: var(--sebas-radius-full);
      padding: 0 7px;
    }
    .entry button {
      background: none;
      border: none;
      color: var(--sebas-text-faint);
      cursor: pointer;
      font-size: 12px;
      line-height: 1;
      padding: 2px 4px;
      border-radius: var(--sebas-radius-sm);
    }
    .entry button:hover { color: var(--sebas-text-bright); background: var(--sebas-surface-3); }
    .entry button:focus-visible { outline: var(--sebas-focus-ring); outline-offset: 1px; }
    .entry button.remove:hover { color: var(--sebas-status-failed); }
    .notice {
      margin-bottom: 6px;
      padding: 8px 10px;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-status-failed-bg);
      border: 1px solid var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
      font-size: 0.78rem;
      line-height: 1.4;
    }
    .notice ul { margin: 4px 0 0; padding-left: 18px; }
  `

  /** 展示视图：乐观态优先。 */
  private get view(): PendingSubmission[] {
    return this.optimistic ?? this.pending
  }

  /** turn 组内 1 基位置（文案「待执行 · 第 N 位」）。 */
  private turnOrdinal(p: PendingSubmission): number {
    return this.view.filter((x) => x.disposition === 'turn').indexOf(p) + 1
  }

  /**
   * 条目的处置词（fix-pending-queue-liveness 3.2）：队列被占用时 turn 条目
   * 点名阻塞条件（泊车审批 → 「等待你的审批」；普通在跑回合 → 「等待当前
   * 回合结束」）并附起等时刻；空闲时维持位置词。staging 条目语义不变。
   */
  private dispositionText(p: PendingSubmission): string {
    if (p.disposition === 'staging') return '将并入首条消息'
    if (!this.turnEngaged || this.blockedSince === null) {
      return `待执行 · 第 ${this.turnOrdinal(p)} 位`
    }
    const wait = `已等 ${formatWaitDuration(Date.now() - this.blockedSince)}`
    if (this.waitingApproval) return `等待你的审批 · ${wait}`
    return `等待当前回合结束 · 第 ${this.turnOrdinal(p)} 位 · ${wait}`
  }

  private dispatchChanged(): void {
    this.dispatchEvent(
      new CustomEvent('pending-changed', { bubbles: true, composed: true }),
    )
  }

  /** 静默对账：放弃乐观态并请求一次全量刷新（服务端真相经 prop 回到组件）。 */
  private reconcileSilently(): void {
    this.optimistic = null
    this.busyId = null
    window.dispatchEvent(new CustomEvent('sebas:refetch'))
  }

  /**
   * 两级判据的失败半边（fix-pending-queue-liveness 3.3，design D4）：请求
   * 失败（4xx / 网络失败）后取一次服务端 post-op 真相——条目已不在 = 竞态
   * 竞输（如该提交已开跑），静默对账；条目仍在 / 真相取不到（网络失败归入
   * 确定性拒绝）= 低档通知点名条目与原因。
   */
  private async handleFailure(
    key: string,
    p: PendingSubmission,
    op: string,
    err: unknown,
  ): Promise<void> {
    let postList: PendingSubmission[] | null = null
    try {
      postList = (await api.session(key)).pending
    } catch {
      postList = null
    }
    if (postList !== null && !postList.some((x) => x.id === p.id)) {
      this.reconcileSilently()
      return
    }
    this.reportRejection(p, op, describeError(err))
  }

  /** 确定性拒绝：低档（warn）就地短暂呈现，点名条目文本与原因；随后对账。 */
  private reportRejection(p: PendingSubmission, op: string, reason: string): void {
    this.reconcileSilently()
    notify({
      level: 'warn',
      message: `${op}未生效：「${quoteText(p.text)}」（${reason}）`,
      dedupeKey: `pending:${op}:${p.id}`,
    })
  }

  private async removeEntry(e: Event, p: PendingSubmission): Promise<void> {
    e.stopPropagation()
    const key = this.sessionKey
    if (!key || this.busyId !== null) return
    // 先判后动的乐观更新：仅当条目仍在当前视图时收起它。
    if (!this.view.some((x) => x.id === p.id)) return
    this.busyId = p.id
    this.optimistic = this.view.filter((x) => x.id !== p.id)
    try {
      const { pending } = await api.removePending(key, p.id)
      if (pending.some((x) => x.id === p.id)) {
        // 2xx 但条目仍在服务端全量里 = 服务端没有执行该操作（确定性拒绝，
        // spec「the entry still present after the operation」）。
        this.reportRejection(p, '移除', '服务端未执行该移除')
      } else {
        this.optimistic = pending
        this.dispatchChanged()
      }
    } catch (err) {
      await this.handleFailure(key, p, '移除', err)
    } finally {
      this.busyId = null
    }
  }

  /**
   * 组内移动（键盘 ↑/↓ 与拖拽共用）。`toIndex` 是条目在其处置组内的
   * 0 基插入位。非优先项的目标位若落在优先项之前 → 不发请求、不更新
   * （spec「先判后动」）；服务端拒绝/未按意图收敛按两级判据处理。
   */
  private async move(e: Event, p: PendingSubmission, toIndex: number): Promise<void> {
    e.stopPropagation()
    const key = this.sessionKey
    if (!key || this.busyId !== null) return
    if (toIndex < 0) return
    const group = this.view.filter((x) => x.disposition === p.disposition)
    if (toIndex >= group.length) return
    // 先判：非优先项不得越过优先项（优先项恒为 turn 组前缀）。
    if (!p.priority && p.disposition === 'turn') {
      const priorityCount = group.filter((x) => x.priority).length
      if (toIndex < priorityCount) return
    }
    const from = group.indexOf(p)
    if (from === toIndex) return
    // 乐观重排（同组内 splice）。
    const next = [...this.view]
    const pos = next.indexOf(p)
    next.splice(pos, 1)
    const anchor = group[toIndex]
    const insertAt = anchor === undefined ? next.length : next.indexOf(anchor)
    next.splice(insertAt, 0, p)
    this.optimistic = next.map((x, i) => ({ ...x, position: i }))
    this.busyId = p.id
    try {
      const { pending } = await api.movePending(key, p.id, toIndex)
      if (!pending.some((x) => x.id === p.id)) {
        // 竞态竞输：条目已被并发开始消费（不在服务端全量里）——静默对账。
        this.optimistic = pending
        this.dispatchChanged()
        return
      }
      const got = pending
        .filter((x) => x.disposition === p.disposition)
        .findIndex((x) => x.id === p.id)
      if (got === toIndex) {
        this.optimistic = pending
        this.dispatchChanged()
      } else {
        this.reportRejection(p, '重排', '服务端未按意图重排该条目')
      }
    } catch (err) {
      await this.handleFailure(key, p, '重排', err)
    } finally {
      this.busyId = null
    }
  }

  // ─── HTML5 拖拽（仅非优先项；组内落点换算，先判后动）───────────────
  private onDragStart(e: DragEvent, p: PendingSubmission): void {
    if (p.priority) return
    this.dragId = p.id
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move'
      e.dataTransfer.setData('text/plain', String(p.id))
    }
  }
  private onDragOver(e: DragEvent): void {
    if (this.dragId === null) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
    ;(e.currentTarget as HTMLElement).classList.add('drag-over')
  }
  private onDragLeave(e: Event): void {
    ;(e.currentTarget as HTMLElement).classList.remove('drag-over')
  }
  private onDrop(e: DragEvent, target: PendingSubmission): void {
    e.preventDefault()
    ;(e.currentTarget as HTMLElement).classList.remove('drag-over')
    const dragged = this.view.find((x) => x.id === this.dragId)
    this.dragId = null
    if (!dragged || dragged.disposition !== target.disposition) return
    // 落点换算：目标条目在组内的位置即组内 to_index。
    const group = this.view.filter((x) => x.disposition === dragged.disposition)
    this.move(e, dragged, group.indexOf(target)).catch(() => {})
  }
  private onDragEnd(): void {
    this.dragId = null
  }

  render() {
    const notice = this.dropped !== null && this.dropped.length > 0
    const hasStack = this.sessionKey !== null && this.view.length > 0
    if (!notice && !hasStack) return nothing
    return html`
      ${notice
        ? html`
            <div class="notice" role="alert" data-testid="pending-dropped-notice">
              <span>以下待执行消息未被执行（会话已结束）：</span>
              <ul>
                ${this.dropped!.map(
                  (d) =>
                    html`<li>
                      ${d.text}
                      ${d.priority ? html`<span class="prio">/btw</span>` : nothing}
                    </li>`,
                )}
              </ul>
            </div>
          `
        : nothing}
      ${hasStack
        ? html`
            <div class="stack" aria-label="待执行的提交" data-testid="pending-stack">
              <span class="stack-title">待执行 · ${this.view.length}</span>
              ${this.view.map(
                (p) => html`
                  <div
                    class=${[
                      'entry',
                      p.priority ? '' : 'draggable',
                      this.dragId === p.id ? 'dragging' : '',
                      this.busyId === p.id ? 'busy' : '',
                    ]
                      .filter(Boolean)
                      .join(' ')}
                    draggable=${!p.priority && this.busyId !== p.id ? 'true' : 'false'}
                    @dragstart=${(e: DragEvent) => this.onDragStart(e, p)}
                    @dragover=${(e: DragEvent) => this.onDragOver(e)}
                    @dragleave=${this.onDragLeave}
                    @drop=${(e: DragEvent) => this.onDrop(e, p)}
                    @dragend=${this.onDragEnd}
                  >
                    ${p.priority ? html`<span class="prio" title="优先（/btw）">/btw</span>` : nothing}
                    <span class="text" title=${p.text}>${p.text}</span>
                    <span class="disp" data-testid=${`pending-disposition-${p.id}`}>
                      ${this.dispositionText(p)}
                    </span>
                    ${p.priority
                      ? nothing
                      : html`
                          <button
                            class="mv-up"
                            aria-label="上移 ${p.text}"
                            title="上移"
                            @click=${(e: Event) =>
                              this.move(
                                e,
                                p,
                                this.groupIndex(p) - 1,
                              )}
                          >
                            ↑
                          </button>
                          <button
                            class="mv-down"
                            aria-label="下移 ${p.text}"
                            title="下移"
                            @click=${(e: Event) =>
                              this.move(
                                e,
                                p,
                                this.groupIndex(p) + 1,
                              )}
                          >
                            ↓
                          </button>
                        `}
                    <button
                      class="remove"
                      aria-label="移除 ${p.text}"
                      title="移除"
                      @click=${(e: Event) => this.removeEntry(e, p)}
                    >
                      ×
                    </button>
                  </div>
                `,
              )}
            </div>
          `
        : nothing}
    `
  }

  /** 条目在其处置组内的 0 基下标（键盘移动的当前位置）。 */
  private groupIndex(p: PendingSubmission): number {
    return this.view.filter((x) => x.disposition === p.disposition).indexOf(p)
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-pending-stack': SebasPendingStack
  }
}

/** 通知文案里的条目文本截断（点名但不刷屏）。 */
function quoteText(text: string): string {
  return text.length > 40 ? `${text.slice(0, 40)}…` : text
}

/**
 * 失败的呈现原因词（fix-pending-queue-liveness 3.3）：ApiError 携带服务端
 * 的类型化拒绝文案（「待执行提交不存在」/「该提交已开始执行」…）；网络级
 * 失败如实说「无法确认」——两级判据里它归入确定性拒绝（宁可误报不可无感）。
 */
function describeError(err: unknown): string {
  if (err instanceof ApiError) return err.message
  if (err instanceof NetworkError) return '网络失败，原因无法确认'
  return String(err)
}

/** 起等时长的呈现分桶（秒 → 分钟 → 小时，不假精度）。 */
export function formatWaitDuration(ms: number): string {
  const secs = Math.max(0, Math.floor(ms / 1000))
  if (secs < 60) return `${secs} 秒`
  if (secs < 3600) return `${Math.floor(secs / 60)} 分钟`
  return `${Math.floor(secs / 3600)} 小时`
}
