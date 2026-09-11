/**
 * 待生效堆叠区（workbench-turn-queue 7.1–7.3，design D8）。
 *
 * 渲染在 composer 上方、投递序列出聚焦会话的待生效提交（pending
 * submissions），不占 transcript 空间。每个条目声明处置：
 *   - staging → 「将并入首条消息」（spawn 窗口暂存，激活时合并）；
 *   - turn → 「待执行 · 第 N 位」（N 为组内 1 基位置）。
 *
 * 交互面：
 *   - 逐条删除（× 按钮）；
 *   - HTML5 拖拽重排（仅同处置组内；优先项 /btw 不可拖、不可被越过——
 *     「先判后动」：非法落点不发请求、不做乐观更新）；
 *   - 键盘可达的上移/下移按钮（纯拖拽不可达，仓库有 a11y 门禁）。
 *
 * 对账（design D8）：操作先乐观更新，随即以服务端返回的全量 pending
 * 重建列表；`AlreadyStarted`（提交已开跑）等拒绝一律静默刷新，绝不弹错
 * ——服务端真相经 refetch 事件回到组件。
 *
 * 会话终结（7.3）：`dropped` 非空时渲染一次性「未执行」提示，逐条点名
 * 被丢弃的提交——堆叠区随会话消失，提示是唯一记录。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, type PendingSubmission } from '../api/client.js'

@customElement('sebas-pending-stack')
export class SebasPendingStack extends LitElement {
  /** 聚焦会话的待生效提交（投递序，来自会话 payload）。 */
  @property({ attribute: false }) pending: PendingSubmission[] = []
  /** 会话的 encoded key；null = 无聚焦会话（堆叠区隐藏）。 */
  @property({ attribute: false }) sessionKey: string | null = null
  /** （7.3）会话终结时被丢弃的提交；null = 无提示。非空数组渲染一次性提示。 */
  @property({ attribute: false }) dropped: PendingSubmission[] | null = null

  /** 乐观视图：操作在途时覆盖展示，成功即被服务端返回值清空。 */
  @state() private optimistic: PendingSubmission[] | null = null
  /** 操作在途的条目 id（禁用其再次拖拽/操作）。 */
  @state() private busyId: number | null = null
  /** HTML5 拖拽进行中的条目 id。 */
  @state() private dragId: number | null = null

  protected willUpdate(changed: Map<string, unknown>): void {
    // 服务端对账后的下一次全量刷新（prop 更新）接管真相，撤销乐观覆盖。
    if (changed.has('pending')) this.optimistic = null
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

  private dispatchChanged(): void {
    this.dispatchEvent(
      new CustomEvent('pending-changed', { bubbles: true, composed: true }),
    )
  }

  /** 静默对账（design D8）：拒绝即放弃乐观态并请求一次全量刷新。 */
  private reconcileSilently(): void {
    this.optimistic = null
    this.busyId = null
    window.dispatchEvent(new CustomEvent('sebas:refetch'))
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
      this.optimistic = pending
      this.dispatchChanged()
    } catch {
      this.reconcileSilently()
    } finally {
      this.busyId = null
    }
  }

  /**
   * 组内移动（键盘 ↑/↓ 与拖拽共用）。`toIndex` 是条目在其处置组内的
   * 0 基插入位。非优先项的目标位若落在优先项之前 → 不发请求、不更新
   * （spec「先判后动」）；服务端仍可能拒绝（竞态），按静默对账处理。
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
      this.optimistic = pending
      this.dispatchChanged()
    } catch {
      this.reconcileSilently()
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
                    <span class="disp">
                      ${p.disposition === 'staging'
                        ? '将并入首条消息'
                        : `待执行 · 第 ${this.turnOrdinal(p)} 位`}
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
