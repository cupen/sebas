/**
 * Review cards: the operator surface for gated tool calls. Rendering has a
 * SINGLE store entry (round3 2.2, design decision 3): both the read model
 * (`GET /api/sessions/{key}/approvals`) and the WS `permission.requested`
 * push normalize their rows into `mergeRows` — the push only writes the
 * store, the store is the only thing that renders. Merging is keyed by
 * `request_id` (duplicate frames never create a second card) and decided
 * ids are tombstoned (a late push or stale read-model row cannot resurrect
 * a settled card). While the phase feed says the session is `waiting` but
 * the store holds no live card (a lost push), the read model is re-pulled
 * through the same entry so the card recovers without a reload.
 *
 * Answering POSTs the decision to `/api/permissions/{request_id}/answer` —
 * success removes the card, a 404 marks it expired (the pending request is
 * gone server-side: answered, timed out or unknown), any other error keeps
 * the card retryable with the failure surfaced inline.
 *
 * When `sessionKey` is set only frames for that (encoded) session key are
 * rendered — the session-detail view passes its key; `null` (default)
 * renders every request, whatever session raised it.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, ApiError, type PendingApprovalInfo, type PermissionDecision } from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import { icon } from './icons.js'
import { viewStyles } from '../styles/shared.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'

interface ReviewCard {
  request_id: string
  session_id: string
  tool_name: string
  args: unknown
  reason: string
  /**
   * `pending` → buttons live; `answering` → POST in flight, buttons
   * disabled; `expired` → the server no longer holds this request, the
   * card is kept visible but inert.
   */
  state: 'pending' | 'answering' | 'expired'
  /** Last submit failure (transport/server), '' when none. */
  error: string
  /** Reason typed for the one-shot escalate decision. */
  escalateReason: string
}

/**
 * 进入 store 的审批行（round3 2.2）：推送帧与读模型行各自的字段形状归一
 * 到这里（reason 缺省空串），合并语义只有 mergeRows 一处。
 */
interface ApprovalRow {
  request_id: string
  session_id: string
  tool_name: string
  args: unknown
  reason: string
}

@customElement('sebas-review-cards')
export class SebasReviewCards extends LitElement {
  /** Encoded session key to filter on; null renders every request. */
  @property({ attribute: false }) sessionKey: string | null = null

  /**
   * （round3 2.2）聚焦会话的相位词（dashboard 从相位帧/详情下传）。帧说
   * 「waiting」（在等审批）而 store 里没有待决卡 = 推送半边丢了帧——经
   * 读模型重取一次（同一 mergeRows 入口）就地补卡，不必等 reload。只在
   * 相位变化时触发；有卡或读模型为空都无变化，幂等。
   */
  @property({ attribute: false }) sessionPhase: string | null = null

  @state() private cards: ReviewCard[] = []
  private unsubscribe?: () => void

  static styles = [
    viewStyles,
    css`
      :host {
        display: block;
      }
      .review-cards {
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-3);
      }
      .review-card {
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border-strong);
        /* 5.5 / design D5: a pending permission request is "your move" — the
         * --signal accent, not a status hue (statuses describe the machine). */
        border-left: 3px solid var(--sebas-signal);
        border-radius: var(--sebas-radius-md);
        box-shadow: var(--sebas-shadow-1);
        padding: var(--sebas-space-4);
      }
      .review-card[data-state='expired'] {
        border-left-color: var(--sebas-status-dormant);
        opacity: 0.75;
      }
      .review-card .head {
        display: flex;
        align-items: baseline;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
      }
      .review-card .head svg {
        align-self: center;
        color: var(--sebas-signal);
        flex: 0 0 auto;
      }
      .review-card[data-state='expired'] .head svg {
        color: var(--sebas-status-dormant);
      }
      .review-card .head .tool {
        font-family: var(--sebas-font-mono);
        font-weight: 650;
        color: var(--sebas-text-bright);
      }
      .review-card .head .why {
        color: var(--sebas-text-dim);
        font-size: 0.85rem;
      }
      .review-card .meta {
        display: flex;
        gap: var(--sebas-space-3);
        flex-wrap: wrap;
        color: var(--sebas-text-faint);
        font-size: 0.72rem;
        overflow-wrap: anywhere;
      }
      /* The call's arguments: formatted JSON, internally scrollable. */
      .review-card .args {
        margin: 0;
        background: var(--sebas-well, var(--sebas-surface-2));
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-sm);
        padding: var(--sebas-space-2) var(--sebas-space-3);
        font-family: var(--sebas-font-mono);
        font-size: 0.78rem;
        line-height: 1.5;
        overflow-wrap: break-word;
        white-space: pre-wrap;
        max-height: 140px;
        overflow-y: auto;
      }
      .review-card .actions {
        display: flex;
        gap: var(--sebas-space-2);
        flex-wrap: wrap;
        margin-top: var(--sebas-space-1);
      }
      .review-card .escalate {
        display: flex;
        gap: var(--sebas-space-2);
        align-items: center;
      }
      .review-card .escalate wa-input {
        flex: 1;
        min-width: 180px;
      }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    this.unsubscribe = sharedWs.subscribe((event) => {
      if (event.type !== 'permission.requested') return
      if (this.sessionKey && event.session_id !== this.sessionKey) return
      // （round3 2.2）推送只写 store：帧归一为 ApprovalRow 经 mergeRows
      // 单一入口合并——与读模型重建同一渲染入口，不再有与重建并行的独立
      // 推送渲染分支。mergeRows 负责 request_id 去重与已决墓碑（广播可
      // 重放，一张卡只建一次；过期卡留在列表里，迟到重放无法复活）。
      this.mergeRows([
        {
          request_id: event.request_id,
          session_id: event.session_id,
          tool_name: event.tool_name,
          args: event.args,
          reason: event.reason,
        },
      ])
    })
    // fix-webui-approval-restore-and-session-identity 1.3：组件挂载时
    // sessionKey 已就绪（深链/刷新直进详情）同样重建一次。
    if (this.sessionKey) void this.pullApprovals(this.sessionKey)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    super.disconnectedCallback()
  }

  protected willUpdate(changed: Map<string, unknown>): void {
    // Switching the viewed session drops cards collected for the previous
    // one, then rebuilds from the read model
    // （fix-webui-approval-restore-and-session-identity 1.3，design D1）：
    // sessionKey 就绪即拉取一次 `GET /api/sessions/{key}/approvals`，刷新/
    // 重连后审批面不依赖一次性 WS 推送即可重建；后续推送按 request_id 幂等
    // 合并进同一张卡。
    if (changed.has('sessionKey')) {
      this.cards = []
      this.decided.clear()
      this.pullSeq += 1
      if (this.sessionKey) {
        const key = this.sessionKey
        void this.pullApprovals(key)
      }
    }
    // （round3 2.2）相位帧对账：帧在说「waiting」而 store 无待决卡 = 推送
    // 丢帧，经读模型（同一入口）补齐——rail 已亮「等待」而审查卡缺席的
    // 空悬态就此收敛，不再依赖 reload。
    if (changed.has('sessionPhase')) this.reconcileWithPhase()
  }

  /**
   * （round3 2.2）store → 渲染的唯一入口：推送帧与读模型行都归一为
   * ApprovalRow 经这里合并进 cards。已决墓碑优先（迟到推送/陈旧读模型行
   * 都不复活卡片），request_id 去重（重建与推送竞速也只建一张卡）。
   */
  private mergeRows(rows: ApprovalRow[]): void {
    const fresh = rows.filter(
      (r) =>
        !this.decided.has(r.request_id) &&
        !this.cards.some((c) => c.request_id === r.request_id),
    )
    if (fresh.length === 0) return
    this.cards = [
      ...this.cards,
      ...fresh.map((r) => ({
        ...r,
        state: 'pending' as const,
        error: '',
        escalateReason: '',
      })),
    ]
  }

  /** 相位帧 → store 对账（round3 2.2）：waiting 且无待决卡才重取读模型。 */
  private reconcileWithPhase(): void {
    if (this.sessionPhase !== 'waiting' || !this.sessionKey) return
    if (this.cards.some((c) => c.state !== 'expired')) return
    const key = this.sessionKey
    void this.pullApprovals(key)
  }

  /**
   * 读模型重建（1.3 + round3 2.2）：拉取当前泊车审批，行归一为 ApprovalRow
   * 后经 mergeRows 单一入口合并。失败静默降级——读模型不可得时审批面仍由
   * 推送通道承载。代际核对：响应回来时 sessionKey 已切换则丢弃。
   */
  private pullSeq = 0
  private async pullApprovals(sessionKey: string): Promise<void> {
    const seq = ++this.pullSeq
    let approvals: PendingApprovalInfo[]
    try {
      ;({ approvals } = await api.sessionApprovals(sessionKey))
    } catch {
      return
    }
    if (seq !== this.pullSeq || this.sessionKey !== sessionKey) return
    this.mergeRows(
      approvals.map((a) => ({
        request_id: a.request_id,
        session_id: sessionKey,
        tool_name: a.tool_name,
        args: a.args,
        reason: '',
      })),
    )
  }

  /** 已决 request_id 墓碑：批复成功后到达的同 id 推送/读模型行不再复活卡片。 */
  private decided = new Set<string>()

  private patch(requestId: string, patch: Partial<ReviewCard>): void {
    this.cards = this.cards.map((c) => (c.request_id === requestId ? { ...c, ...patch } : c))
  }

  private async answer(card: ReviewCard, decision: PermissionDecision): Promise<void> {
    if (card.state !== 'pending') return
    this.patch(card.request_id, { state: 'answering', error: '' })
    try {
      await api.answerPermission(card.request_id, decision)
      // Delivered: the card has done its job. Tombstone the id so a late
      // push or a stale read-model row cannot resurrect it
      // （fix-webui-approval-restore-and-session-identity 1.3）.
      this.decided.add(card.request_id)
      this.cards = this.cards.filter((c) => c.request_id !== card.request_id)
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        // No pending request with that id — answered elsewhere, timed out
        // or unknown. Mark the card expired; it stays visible but inert.
        this.patch(card.request_id, { state: 'expired', error: '' })
      } else {
        // Transport/server hiccup: keep the card retryable.
        this.patch(card.request_id, { state: 'pending', error: String(e) })
      }
    }
  }

  private formatArgs(args: unknown): string {
    try {
      return JSON.stringify(args ?? {}, null, 2)
    } catch {
      return String(args)
    }
  }

  private renderCard(card: ReviewCard) {
    const busy = card.state !== 'pending'
    const escalateReason = card.escalateReason.trim()
    return html`
      <section class="review-card" data-request-id=${card.request_id} data-state=${card.state}>
        <header class="head">
          ${icon('shield', 16)}
          <span class="tool">${card.tool_name}</span>
          ${card.reason ? html`<span class="why">${card.reason}</span>` : nothing}
        </header>
        <div class="meta">
          <span class="session-id" title=${`session ${card.session_id}`}
            >session ${card.session_id}</span
          >
          <span class="request-id" title=${`request ${card.request_id}`}
            >request ${card.request_id}</span
          >
        </div>
        <pre class="args">${this.formatArgs(card.args)}</pre>
        ${card.state === 'expired'
          ? html`<div class="callout callout-warning" role="status">
              ${icon('alert')}<span
                >No longer pending — already answered, timed out or cleared.</span
              >
            </div>`
          : html`
              <div class="actions">
                <wa-button
                  size="s"
                  class="allow-once"
                  variant="brand"
                  appearance="accent"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'allow_once' })}
                  >Allow once</wa-button
                >
                <wa-button
                  size="s"
                  class="allow-session"
                  variant="success"
                  appearance="outlined"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'allow_session' })}
                  >Allow for session</wa-button
                >
                <wa-button
                  size="s"
                  class="deny"
                  variant="danger"
                  appearance="outlined"
                  ?disabled=${busy}
                  @click=${() => void this.answer(card, { decision: 'deny' })}
                  >Deny</wa-button
                >
              </div>
              <div class="escalate">
                <wa-input
                  class="escalate-reason"
                  size="s"
                  placeholder="Why raise this once? (escalate)"
                  aria-label="Escalation reason"
                  .value=${card.escalateReason}
                  ?disabled=${busy}
                  @input=${(e: Event) =>
                    this.patch(card.request_id, {
                      escalateReason: (e.target as HTMLInputElement).value,
                    })}
                ></wa-input>
                <wa-button
                  size="s"
                  class="escalate"
                  variant="warning"
                  appearance="outlined"
                  ?disabled=${busy || escalateReason === ''}
                  @click=${() =>
                    void this.answer(card, {
                      decision: 'escalate',
                      reason: escalateReason,
                    })}
                  >Escalate</wa-button
                >
              </div>
              ${card.error
                ? html`<div class="callout callout-error" role="alert">
                    ${icon('alert')}<span>${card.error}</span>
                  </div>`
                : nothing}
            `}
      </section>
    `
  }

  render() {
    if (this.cards.length === 0) return nothing
    return html`
      <div class="review-cards" role="region" aria-label="Permission review">
        ${this.cards.map((card) => this.renderCard(card))}
      </div>
    `
  }

  protected updated(changed: Map<string, unknown>): void {
    // fix-pending-queue-liveness 3.2：把「聚焦会话在等操作者批复」的事实
    // 上报给宿主（dashboard 据此点亮 composer 的「等待你的审批」指示与
    // 待执行栈的阻塞原因）。只数待决卡（expired 不算——已无人可答）；计数
    // 变化即发（含清零与 sessionKey 切换后的重算），宿主幂等消费。
    if (!changed.has('cards') && !changed.has('sessionKey')) return
    const count = this.cards.filter((c) => c.state !== 'expired').length
    if (count === this.lastReportedPending) return
    this.lastReportedPending = count
    this.dispatchEvent(
      new CustomEvent('review-pending-changed', {
        detail: { count },
        bubbles: true,
        composed: true,
      }),
    )
  }

  /** 上次上报的待决计数（-1 = 尚未上报，0 也要发一次以对齐宿主）。 */
  private lastReportedPending = -1
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-review-cards': SebasReviewCards
  }
}
