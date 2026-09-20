/**
 * Sidebar project tree (app-shell 左侧栏, IA v2 对齐预览原型 preview-app.ts)。
 *
 * rail-declutter-unread：行操作收敛——项目行 = `…` 菜单（移除）+ `+` 新建；
 * 会话行 = 单个 `…` 菜单（归档 / 关闭，active 会话关闭需确认）。按钮默认
 * 隐藏，hover / focus-within 显现（沿用 .row-action 既有 CSS 契约）。会话
 * 行渲染未读徽标（服务端 msg_count − 共享读锚，聚焦清零，见 unread-cursor）。
 * 会话名改用首条用户消息预览（prompt_preview，40 码点截断，title 挂全文）。
 * 分支名不再显示（可达性探测保留，删除线告警不变）。Inbox 分组移除：无
 * 项目会话不再进 rail。History 组按归档时间倒序。
 *
 * wire（workbench-agent-wire-fix）：项目以稳定 id 引用（remove/branch/
 * reorder），会话行带 project_id；path 不再是标识符。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { navigate } from '../router.js'
import {
  api,
  type Project,
  type ProjectBranchInfo,
  type SessionRow,
  type ArchiveEntry,
  type NodeInfo,
} from '../api/client.js'
import { sharedWs } from '../api/shared-ws.js'
import type { WsEvent } from '../api/ws.js'
import { icon } from '../components/icons.js'
import { guardedHide } from '../components/wa-hide-guard.js'
import { unreadCount, writeFocusAnchor } from './unread-cursor.js'
import { COMPOSER_FOCUS_REQUEST } from './workbench-composer.js'
import type { NewSessionDialogConfirm } from './new-session-dialog.js'
import '../components/folder-picker.js'
import './new-session-dialog.js'
import '@awesome.me/webawesome/dist/components/dropdown/dropdown.js'
import '@awesome.me/webawesome/dist/components/dropdown-item/dropdown-item.js'

/** 节点可用性轮询周期（add-remote-execution-node 8.2）：节点回归后**免刷新**
 * 恢复——项目行与「+」的可用态跟着真实状态翻转。兼任 rail-declutter-unread
 * 的徽标兜底刷新（session.updated 不逐条目触发）。 */
const NODE_POLL_MS = 10_000

/** 本机节点标识（与后端 projects::LOCAL_NODE_ID 同一词表）。 */
const LOCAL_NODE = 'local'

/** 会话名显示上限（码点）——超出截断加省略号，title 挂全文（D10）。 */
const NAME_CAP_CODEPOINTS = 40

/** 未读徽标数字封顶（design Open Question：任务内自决为 99+）。 */
const UNREAD_BADGE_CAP = 99

/**
 * （fix-webui-qa-defects-round5 3.2）帧触发行名重取的尾沿防抖窗口：同窗
 * 多帧合并为一次列表重取，队列高频翻转不放大请求量。
 */
export const LABEL_REFRESH_DEBOUNCE_MS = 400

/**
 * rail 切换会话成功后的窗口级聚焦事件（fix-webui-qa-defects 4.1，design
 * D3）：`detail.key` 是 switch 响应的 `active_session_key`。dashboard 监听
 * 后立即节流刷新 summary——焦点指针是每客户端操作面，不走 WS/服务端推送。
 * 独立事件名，不复用语义不同的 `sebas:refetch`（全量 refetch）。
 */
export const RAIL_FOCUS_EVENT = 'sebas:rail-focus'

/**
 * 手填路径的禁用原因（fix-webui-qa-defects 7.2 修复，本 change 5.2 收口为
 * spec：越界**与不存在**都必须给出原因且提交控件禁用——不允许只禁用不解
 * 释）。纯函数：注册/预检错误文案映射为输入框旁可读原因；未知失败返回
 * null（不拦截，留给注册接口的点名报错）。
 */
export function addPathScopeHintFrom(error: string): string | null {
  if (/超出允许范围|超出根目录范围|workspace root/.test(error)) {
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

/**
 * rail 展开态的 localStorage 键（fix-webui-approval-restore-and-session-identity
 * 4.2，design D5）：值为**已展开**的项目路径数组——按路径存，删除的项目恢复
 * 时静默忽略（与项目删除语义一致）。
 */
export const RAIL_EXPANDED_KEY = 'sebas.rail-expanded'

/** 解析持久化的展开态：非法/缺失 JSON → `{}`（全收起）；数组元素取字符串。 */
export function parseRailExpanded(raw: string | null): Record<string, boolean> {
  if (!raw) return {}
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return {}
    const out: Record<string, boolean> = {}
    for (const item of parsed) {
      if (typeof item === 'string' && item) out[item] = true
    }
    return out
  } catch {
    return {}
  }
}

/** 展开态 → 持久化形状（已展开路径数组）。 */
export function serializeRailExpanded(record: Record<string, boolean>): string {
  return JSON.stringify(Object.keys(record).filter((k) => record[k]))
}

/**
 * 项目行的展开判定（4.2）：有持久化记录以记录为准；**无记录**且该会话项目
 * 下有聚焦会话 → 缺省展开（聚焦所在项目不必先点一下才可见）；否则收起。
 * 展开态绝不因刷新/聚焦变化自行翻转——记录存在时聚焦不再改写缺省。
 */
export function railExpandedDefault(
  record: Record<string, boolean>,
  path: string,
  focusedSessionInProject: boolean,
): boolean {
  const recorded = record[path]
  if (recorded !== undefined) return recorded
  return focusedSessionInProject
}

/** 会话名截断：超上限加 `…`（按码点，不切多字节字符）。 */
export function truncateName(label: string, cap = NAME_CAP_CODEPOINTS): string {
  const cps = [...label]
  if (cps.length <= cap) return label
  return cps.slice(0, cap).join('') + '…'
}

/**
 * （round3 4.4）展示路径分隔符统一：注册弹窗的填充值与「项目已注册」等
 * 服务端错误提示里的路径，展示前把反斜杠归一为正斜杠——folder-picker
 * （browse-dirs）回传的是正斜杠普通形，服务端错误消息里是反斜杠普通形，
 * 同一界面两种分隔符混排即 QA 4.4 的「\\ / 混用」。纯展示归一：两种形态
 * 服务端 canonicalize 等价接受，不改变语义。
 */
export function normalizeDisplayPath(text: string): string {
  return text.replace(/\\/g, '/')
}

/**
 * 会话名 = 首条用户消息预览；零轮占位回退短 id / 键尾段（D10）。
 *
 * workbench-interaction-polish 3.2 修复：0-turn 占位（无 prompt、无
 * session_id）在 /api/sessions 行上三者全空（`chat_id` 本就不在该 payload
 * 的词表里）——回退到键的 reference 尾段，`[...undefined]` 曾把整棵 rail
 * 渲染炸掉。模块级导出：/sessions 表格的卡片链接同源复用。
 */
export function fullSessionLabel(row: SessionRow): string {
  // （fix-webui-approval-restore-and-session-identity 5.1，design D6）命名
  // 优先级：label → 首条 prompt 预览 → 短 id → 键尾段。只影响「设置了
  // label」的会话——未设置时行为与旧版本完全一致。
  return (
    row.label ??
    row.prompt_preview ??
    row.session_id_short ??
    row.chat_id ??
    decodeSessionKeyTail(row.encoded_key)
  )
}

/** `web%00web-1709…-4` → `web-1709…-4`（键尾段 = 占位会话的可读短名）。 */
function decodeSessionKeyTail(encodedKey: string): string {
  try {
    const parts = decodeURIComponent(encodedKey).split('\0')
    return parts[parts.length - 1] || encodedKey
  } catch {
    return encodedKey
  }
}

/** unix 秒 → 粗粒度相对时间（离线成因文案用；不引入日期库）。 */
function relativeTime(unixSecs: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - unixSecs)
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`
  return `${Math.floor(diff / 86400)}d ago`
}

@customElement('sebas-project-rail')
export class SebasProjectRail extends LitElement {
  @property({ type: String }) activePath: string | null = null

  @state() private projects: Project[] = []
  @state() private sessions: SessionRow[] = []
  /**
   * 焦点会话指针（workbench-conversation-view 3.2）：/api/sessions 响应的
   * `active_session_key`——rail 的「当前」标记看它，不看 location.pathname。
   */
  @state() private focusedKey: string | null = null
  @state() private archivedSessions: ArchiveEntry[] = []
  @state() private expanded: Record<string, boolean> = {}
  @state() private historyOpen = false
  /** 8.4：等待组默认展开（它就是要你看见）。 */
  @state() private waitingOpen = true
  @state() private branchByPath: Record<string, ProjectBranchInfo> = {}
  @state() private dragIndex: number | null = null
  @state() private dragOverIndex: number | null = null
  @state() private error: string | null = null
  /**
   * 项目注册降级提示（harden-core-channel-deployment 4.3/D7）：核心不可达时
   * 注册落本地注册表，就地提示如实文案；core 恢复后由任一次成功 refresh
   * （ws refetch / 重试）清除。
   */
  @state() private degradedHint: string | null = null

  // Add project dialog state
  @state() private addDialogOpen = false
  @state() private addPath = ''
  @state() private addError: string | null = null
  /** fix-webui-qa-defects 7.2：手填路径越界的禁用原因（null = 界内/未知）。 */
  @state() private addPathScopeHint: string | null = null
  /** 注册对话框选定的执行节点（`''` = 本机，隐式；8.1）。 */
  @state() private addNodeId = ''

  // ─── 执行节点可用性（add-remote-execution-node 8.2/8.5）──────────────
  /** `GET /api/nodes` 的节点列表（本机恒在列）。 */
  @state() private nodes: NodeInfo[] = []
  /** 远端注册表是否可得：`false` = 状态未知，**不**等于「没有远端节点」。 */
  @state() private remoteNodesAvailable = true
  @state() private nodesCause: string | null = null

  // Remove project dialog state（workbench-agent-wire-fix 5.1）
  @state() private removeTarget: Project | null = null
  @state() private removeError: string | null = null
  @state() private removing = false

  // ─── New session dialog（workbench-interaction-polish 3.2/D2）──────────
  /** 对话框当前绑定的项目（`null` = 关闭）。唯一创建入口：项目行「+」。 */
  @state() private newSessionTarget: Project | null = null
  /** 创建请求在途（防双击重复创建；round3 1.3 起同时下传对话框忙态）。 */
  @state() private creatingSession = false
  /** 创建失败：留在对话框内就地呈现。 */
  @state() private newSessionError: string | null = null

  private fetchSeq = 0
  private unsubscribe?: () => void
  /** 节点可用性轮询定时器（8.2；disconnectedCallback 清理）。 */
  private nodeTimer: number | undefined = undefined
  private refetchBound = (): void => { void this.refresh() }

  /**
   * （session-parallel-liveness-and-unread-polish 2.2，design D2）会话相位
   * 帧 → 行就地补丁：rail 圆点（status_slug）与未读徽标（msg_count）从帧
   * 字段真读，不等 HTTP 详情轮询、不做任何字符串回退。FSM 每个 flip 的
   * 帧到达即重渲染（七词圆点 + 徽标免刷新更新）。其余事件维持整表收敛
   * 刷新（低频）；turn.append 不刷新——徽标口径只数可见段，段数随下一
   * 条 session.updated 帧或 10s 轮询兜底到达，绝不逐 delta 打 HTTP。
   */
  private onWsEvent = (ev: WsEvent): void => {
    if (ev.type === 'session.updated' || ev.type === 'session.created') {
      const patch = (row: SessionRow): SessionRow =>
        row.encoded_key === ev.session_id
          ? {
              ...row,
              status_slug: ev.status_slug as SessionRow['status_slug'],
              turn_engaged: ev.turn_engaged,
              msg_count: ev.msg_count,
              pending_count: ev.pending.length,
            }
          : row
      this.sessions = this.sessions.map(patch)
      // （fix-webui-qa-defects-round5 3.2，design 决策 4）相位帧是五键定形、
      // 不携带 label——任意路径（rail 对话框 / API）的 label 写入成功都发
      // Updated，行名的命名来源可能随该帧变化：防抖后做一次轻量列表重取，
      // 用返回的 label 重渲染行名（不做全列表轮询）。
      this.scheduleLabelRefresh()
      return
    }
    if (
      ev.type === 'session.removed' ||
      ev.type === 'session.pending_dropped' ||
      ev.type === 'session.turn_stalled'
    ) {
      void this.refresh()
    }
  }

  /**
   * （fix-webui-qa-defects-round5 3.2）帧触发的行名重取：队列高频翻转时
   * session.updated 可能连发——尾沿防抖把一个窗口内的多帧合并为一次既有
   * `GET /api/sessions` 重取（行投影里 label 的唯一来源；单会话 detail 带
   * 聚焦副作用且不带 label，不用）。量级与 10s 轮询兜底相当。
   */
  private labelRefreshTimer: number | undefined = undefined
  private scheduleLabelRefresh(): void {
    if (this.labelRefreshTimer !== undefined) window.clearTimeout(this.labelRefreshTimer)
    this.labelRefreshTimer = window.setTimeout(() => {
      this.labelRefreshTimer = undefined
      void this.refresh()
    }, LABEL_REFRESH_DEBOUNCE_MS)
  }

  static styles = css`
    :host { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
    /* （fix-webui-qa-defects-round5 4.1）菜单关闭态对辅助技术隐藏：wa-dropdown
       的 open 反射属性关闭时菜单项 display:none，a11y 树不再暴露「重命名/
       归档/移除项目」。open 翻转（dropdown updated 内）与 popup 激活同帧，
       打开即恢复可见；菜单项是本树的光 DOM 子孙，宿主样式表可直接命中。 */
    wa-dropdown:not([open]) wa-dropdown-item { display: none; }
    .section-label {
      display: flex; align-items: center; gap: 6px;
      padding: var(--sebas-space-2) 8px var(--sebas-space-1);
      font-size: 0.7rem; font-weight: 600; text-transform: uppercase;
      letter-spacing: 0.08em; color: var(--sebas-text-faint);
    }
    .section-label .add-btn {
      margin-left: auto;
      background: var(--sebas-accent-strong); border: none;
      border-radius: var(--sebas-radius-sm);
      color: var(--sebas-accent-ink); cursor: pointer;
      padding: 0;
      display: grid; place-items: center; width: 22px; height: 22px;
      transition: opacity var(--sebas-dur) var(--sebas-ease), filter var(--sebas-dur) var(--sebas-ease);
    }
    .section-label .add-btn:hover { filter: brightness(1.15); }
    .section-label .add-btn:focus-visible { outline: var(--sebas-focus-ring); outline-offset: 1px; }
    ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
    .degraded-hint {
      margin: 2px 8px 4px;
      padding: 5px 8px;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-status-failed-bg);
      border: 1px solid var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
      font-size: 0.72rem;
      line-height: 1.35;
    }
    .row {
      position: relative; display: grid;
      grid-template-columns: minmax(0, 1fr) auto auto;
      gap: 6px; align-items: center; padding: 6px 10px;
      border-radius: var(--sebas-radius-md); font-size: 0.85rem;
      color: var(--sebas-text-dim); cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
      user-select: none;
    }
    .row:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    /* workbench-rail-polish D1：项目行选中 = 中性提亮（surface 提亮一档 +
       文字变亮 + 字重加重），不再借 accent 底——accent 只留给会话行的
       「当前」标记，两态同屏可辨。选中语义不变（主区正在显示的项目）。 */
    .row.active { background: var(--sebas-surface-3); color: var(--sebas-text-bright); }
    .row.active .name { font-weight: 600; }
    .row.dragging { opacity: 0.4; }
    .row.drag-over { box-shadow: inset 0 2px 0 var(--sebas-accent); }
    .chevron { display: inline-grid; place-items: center; width: 10px; color: var(--sebas-text-faint); font-size: 9px; line-height: 1; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .chevron.open { transform: rotate(90deg); }
    .name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; display: inline-flex; align-items: center; gap: 6px; }
    .meta { display: flex; align-items: center; gap: 6px; color: var(--sebas-text-faint); font-size: 0.7rem; }
    .meta .count { background: var(--sebas-surface-2); border-radius: 999px; padding: 1px 7px; font-weight: 500; font-variant-numeric: tabular-nums; }
    /* （D1）旧的项目行选中态计数徽标 accent 覆盖已随中性提亮一并撤除：
       选中项目行上的计数徽标回归同一套中性 pill。 */
    .wait-dot { width: 6px; height: 6px; border-radius: 50%; background: var(--sebas-status-working); display: inline-block; }
    /* 8.5：项目/会话行上的节点标注。本机也显示（spec：命名每个项目所在的
       节点），但只有非在线态才带告警色。 */
    .node-chip {
      font-family: var(--sebas-font-mono); font-size: 0.66rem; font-weight: 500;
      color: var(--sebas-text-faint); background: var(--sebas-surface-2);
      border-radius: var(--sebas-radius-full); padding: 0 6px;
      max-width: 90px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .node-chip[data-node-status='offline'],
    .node-chip[data-node-status='revoked'],
    .node-chip[data-node-status='unknown'] {
      color: var(--sebas-status-failed);
      background: var(--sebas-status-failed-bg);
    }
    .node-cause {
      margin: 2px 10px 4px; padding: 3px 8px;
      border-radius: var(--sebas-radius-sm);
      background: var(--sebas-status-failed-bg);
      border: 1px solid var(--sebas-status-failed-border);
      color: var(--sebas-status-failed);
      font-size: 0.7rem; line-height: 1.35;
    }
    .row.node-offline .name > span:first-child { color: var(--sebas-text-faint); }
    .row-action:disabled { opacity: 0.3; cursor: not-allowed; }
    .row:hover .row-action:disabled { color: var(--sebas-text-faint); border-color: var(--sebas-border); }
    .session-node {
      font-family: var(--sebas-font-mono); font-size: 0.62rem; color: var(--sebas-text-faint);
      max-width: 72px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    li.session-item.waiting .session-name { color: var(--sebas-status-waiting); }
    .wait-badge {
      font-size: 0.62rem; font-weight: 600; letter-spacing: 0.02em;
      color: var(--sebas-status-waiting); background: var(--sebas-status-waiting-bg);
      border: 1px solid var(--sebas-status-waiting-border);
      border-radius: var(--sebas-radius-full); padding: 0 6px; white-space: nowrap;
    }
    .row-actions {
      display: flex; align-items: center; gap: 4px;
    }
    .row-action {
      width: 20px; height: 20px; background: none; border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-sm); color: var(--sebas-text-faint); cursor: pointer;
      display: grid; place-items: center; padding: 0; opacity: 0;
      transition: opacity var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease), background var(--sebas-dur) var(--sebas-ease), border-color var(--sebas-dur) var(--sebas-ease);
    }
    .row-remove:hover { color: var(--sebas-status-failed); background: var(--sebas-status-failed-bg); }
    .row:hover .row-action, .row:focus-within .row-action { opacity: 1; }
    .row:hover .row-action { color: var(--sebas-accent); border-color: var(--sebas-accent-border); }
    .row-action:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
    .row-action:focus-visible { opacity: 1; outline: var(--sebas-focus-ring); outline-offset: 1px; }
    ul.sessions { padding: 0; }
    li.session-item {
      display: flex; align-items: center; gap: 8px; padding: 4px 8px 4px 28px;
      border-radius: var(--sebas-radius-md); color: var(--sebas-text-dim); font-size: 0.8rem;
      cursor: pointer;
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    li.session-item:hover { background: var(--sebas-surface-2); color: var(--sebas-text-bright); }
    /* （2.4，D4）未读行强调：accent-soft 族行底 tint + 名字提亮——在读
       数字之前一行即可分辨（spec「unread rows are prominent」）。:not(.current)
       让「当前会话」的 accent 选中态不被稀释。 */
    li.session-item.unread:not(.current) {
      background: var(--sebas-accent-soft);
      color: var(--sebas-text-bright);
    }
    li.session-item.current { background: var(--sebas-accent-soft); color: var(--sebas-accent); }
    .session-dot { width: 6px; height: 6px; border-radius: 50%; flex: 0 0 auto; background: var(--sebas-text-faint); }
    .session-dot[data-status='starting'] { background: var(--sebas-status-starting); }
    .session-dot[data-status='queued'] { background: var(--sebas-status-queued); }
    .session-dot[data-status='working'] { background: var(--sebas-status-working); }
    .session-dot[data-status='waiting'] { background: var(--sebas-status-waiting); }
    .session-dot[data-status='done'] { background: var(--sebas-status-done); }
    .session-dot[data-status='failed'] { background: var(--sebas-status-failed); }
    .session-dot[data-status='dormant'] { background: var(--sebas-status-dormant); }
    .session-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--sebas-font-mono); font-size: 0.74rem; }
    li.session-item.unreachable .session-name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .row.unreachable .name { text-decoration: line-through; color: var(--sebas-text-faint); }
    .empty { padding: 10px 12px; color: var(--sebas-text-faint); font-size: 0.78rem; }
    /* rail-declutter-unread 2.3：未读徽标——高亮数字（accent 底），99+ 封顶。
       （2.4，D4）数字对比度微调：字重加重 + 字号微升，accent-strong 底上的
       accent-ink 数字一眼可读。 */
    .unread-badge {
      flex: 0 0 auto;
      font-size: 0.66rem; font-weight: 800; line-height: 1.4;
      color: var(--sebas-accent-ink); background: var(--sebas-accent-strong);
      border-radius: var(--sebas-radius-full); padding: 0 7px;
      font-variant-numeric: tabular-nums; white-space: nowrap;
    }
    /* 会话行的「…」菜单触发钮：与项目行共用 .row-action 外观，hover/
       focus-within 显现规则在会话行上等价一份（3.2）。 */
    li.session-item wa-dropdown { display: inline-flex; flex: 0 0 auto; }
    .row-actions wa-dropdown { display: inline-flex; }
    li.session-item:hover .row-action,
    li.session-item:focus-within .row-action,
    .row-action:focus-visible { opacity: 1; }
    li.session-item .row-action:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
    li.session-item.archived { opacity: 0.7; }
    li.session-item.archived:hover { opacity: 1; }
    /* （fix-webui-qa-defects-round4 3.2）History 条目的项目路径段截断省略：
       Windows 绝对路径很长（且分隔符可能未归一），不截断会把 rail 撑出横向
       滚动。min-width:0 + 允许收缩是 flex 子项省略号生效的前提。 */
    .archive-meta {
      font-size: 0.66rem; color: var(--sebas-text-faint); font-family: var(--sebas-font-mono);
      min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .group-section { margin-top: var(--sebas-space-3); }
    .group-head {
      /* （5.5）原生 <button>：语义与键盘行为来自元素本身；重置外观回原 div 视觉。 */
      width: 100%;
      text-align: left;
      background: none;
      border: none;
      font: inherit;
      display: flex; align-items: center; gap: 6px; padding: 4px 8px;
      font-size: 0.66rem; font-weight: 600; text-transform: uppercase;
      letter-spacing: 0.08em; color: var(--sebas-text-faint); cursor: pointer;
      user-select: none; border-radius: var(--sebas-radius-md);
      transition: background var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
    }
    .group-head:hover { background: var(--sebas-surface-2); color: var(--sebas-text-dim); }
    .group-head .chevron { font-size: 9px; transition: transform var(--sebas-dur) var(--sebas-ease); }
    .group-head .chevron.open { transform: rotate(90deg); }
    .group-head .group-count { margin-left: auto; font-variant-numeric: tabular-nums; background: var(--sebas-surface-3); border-radius: var(--sebas-radius-full); padding: 0 7px; font-size: 0.62rem; }
    .group-head:focus-visible { outline: var(--sebas-focus-ring); outline-offset: 1px; }
    .error { padding: 8px 12px; color: var(--sebas-status-failed); font-size: 0.78rem; }
    .error .retry-btn {
      margin-left: 4px;
      padding: 1px 8px;
      border: 1px solid currentColor;
      border-radius: var(--sebas-radius-md);
      background: none;
      color: inherit;
      font: inherit;
      font-size: 0.72rem;
      cursor: pointer;
    }
  `

  connectedCallback(): void {
    super.connectedCallback()
    // （4.2，design D5）展开态从 localStorage 恢复（键不存在/损坏 → 全收起
    // 的既有缺省）；聚焦缺省展开在 isProjectExpanded 的无记录分支生效。
    this.expanded = parseRailExpanded(this.readExpandedStorage())
    void this.refresh()
    this.unsubscribe = sharedWs.subscribe((ev) => this.onWsEvent(ev))
    window.addEventListener('sebas:refetch', this.refetchBound)
    // 8.2：节点离线/回归没有对应的会话事件，靠轮询让项目行与「+」的
    // 可用态**免刷新**翻转。
    this.nodeTimer = window.setInterval(() => { void this.refresh() }, NODE_POLL_MS)
  }

  disconnectedCallback(): void {
    this.unsubscribe?.()
    window.removeEventListener('sebas:refetch', this.refetchBound)
    if (this.nodeTimer !== undefined) {
      this.nodeTimer = window.clearInterval(this.nodeTimer) as unknown as number
      this.nodeTimer = undefined
    }
    if (this.labelRefreshTimer !== undefined) {
      window.clearTimeout(this.labelRefreshTimer)
      this.labelRefreshTimer = undefined
    }
    super.disconnectedCallback()
  }

  async refresh() {
    const seq = ++this.fetchSeq
    // 节点可用性先取（项目行的离线呈现依赖它）。取不到时如实降级为
    // 「状态不可得」，绝不把「看不见」说成「离线」，更不说成「在线」。
    try {
      const d = await api.nodes()
      if (seq !== this.fetchSeq) return
      this.nodes = d?.nodes ?? []
      this.remoteNodesAvailable = d?.remote_available !== false
      this.nodesCause = d?.cause ?? null
    } catch (e) {
      if (seq !== this.fetchSeq) return
      this.nodes = [{ id: LOCAL_NODE, status: 'online', local: true }]
      this.remoteNodesAvailable = false
      this.nodesCause = e instanceof Error ? e.message : String(e)
    }
    try {
      const { projects } = await api.projects.list()
      if (seq !== this.fetchSeq) return
      this.projects = projects
      this.error = null
      this.degradedHint = null
      for (const p of projects) {
        if (!this.branchByPath[p.id]) void this.loadBranch(p.id)
      }
    } catch (e) {
      if (seq !== this.fetchSeq) return
      this.error = e instanceof Error ? e.message : String(e)
    }
    try {
      const list = await api.sessions()
      if (seq !== this.fetchSeq) return
      this.sessions = list.recent_sessions
      this.focusedKey = list.active_session_key
      // （4.2）聚焦缺省展开的**物化**：无记录且其下有聚焦会话的项目，此刻
      // 把展开写进记录（随写持久化）。物化让缺省成为显式状态——之后的聚焦
      // 变化/刷新绝不翻转它（「无操作不自行收起」），toggle 也从确定基数
      // 起步，而不是从随焦点漂移的派生值起步。
      let materialized = false
      for (const p of this.projects) {
        if (this.expanded[p.path] === undefined && this.focusedInProject(p.path)) {
          this.expanded = { ...this.expanded, [p.path]: true }
          materialized = true
        }
      }
      if (materialized) this.writeExpandedStorage()
    } catch { /* ignore */ }
    try {
      const { archived_sessions } = await api.archiveList()
      if (seq !== this.fetchSeq) return
      this.archivedSessions = archived_sessions
    } catch { /* ignore */ }
  }

  /**
   * 一个节点的可判定状态（8.2/8.5）。返回 `status` ∈ online | offline |
   * revoked | unknown，以及不可用时的**成因**。
   *
   * `unknown` 是真实的一档：注册表不可得时我们不知道那台机器通不通，不能
   * 说它离线，也不能默认它在线。本机节点例外——本机在回答这个页面。
   */
  private nodeStatus(nodeId: string | null | undefined): { status: string; cause: string | null } {
    const id = nodeId || LOCAL_NODE
    const found = this.nodes.find((n) => n.id === id)
    if (found) {
      if (found.status === 'online') return { status: 'online', cause: null }
      if (found.status === 'revoked') return { status: 'revoked', cause: '节点凭据已被吊销' }
      const seen = found.last_seen_unix
      return {
        status: found.status,
        cause: seen ? `节点离线（上次在线 ${relativeTime(seen)}）` : '节点离线',
      }
    }
    if (id === LOCAL_NODE) return { status: 'online', cause: null }
    if (!this.remoteNodesAvailable) {
      return {
        status: 'unknown',
        cause: `节点状态不可得${this.nodesCause ? `：${this.nodesCause}` : ''}`,
      }
    }
    return { status: 'unknown', cause: `节点 ${id} 未注册` }
  }

  /** 项目是否注册在一个**可建会话**的节点上（online 才可）。 */
  private nodeOnline(nodeId: string | null | undefined): boolean {
    return this.nodeStatus(nodeId).status === 'online'
  }

  private async loadBranch(id: string) {
    try {
      const info = await api.projects.branch(id)
      this.branchByPath = { ...this.branchByPath, [id]: info }
    } catch { /* 404 = removed mid-flight */ }
  }

  private onSelect(path: string) {
    this.expanded = { ...this.expanded, [path]: !this.isProjectExpanded(path) }
    // （4.2）写入时机 = toggle：持久化让展开跨刷新保持（只写，不联动
    // rail-select 的主区项目选择语义）。
    this.writeExpandedStorage()
    this.dispatchEvent(new CustomEvent('rail-select', { detail: { path }, bubbles: true, composed: true }))
  }

  /** 项目行的展开判定（4.2）：持久化记录优先，无记录时聚焦所在项目缺省展开。 */
  private isProjectExpanded(path: string): boolean {
    return railExpandedDefault(this.expanded, path, this.focusedInProject(path))
  }

  /** 聚焦会话是否落在给定路径的项目下（rail 已知 active key 时才有意义）。 */
  private focusedInProject(path: string): boolean {
    if (this.focusedKey === null) return false
    const project = this.projects.find((p) => p.path === path)
    if (!project) return false
    return this.sessions.some(
      (r) => r.encoded_key === this.focusedKey && r.project_id === project.id,
    )
  }

  private readExpandedStorage(): string | null {
    try {
      return window.localStorage.getItem(RAIL_EXPANDED_KEY)
    } catch {
      return null
    }
  }

  private writeExpandedStorage(): void {
    try {
      window.localStorage.setItem(RAIL_EXPANDED_KEY, serializeRailExpanded(this.expanded))
    } catch {
      /* 隐私模式等 localStorage 不可用：展开态退化为纯内存（现状行为）。 */
    }
  }

  /**
   * 点会话 = switch + 就地聚焦（workbench-conversation-view 3.1，design
   * D6）：POST switch 设置服务端焦点指针后停在 `/`，dashboard 就地渲染该
   * 会话——不再 navigate 到深链离开工作台。switch 404（会话恰好被关闭）
   * 时只刷新列表，不导航。
   */
  private async openSession(row: SessionRow) {
    let resp: { status: string; redirect: string; active_session_key: string } | undefined
    try {
      resp = await api.switchSession(row.encoded_key)
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err)
      void this.refresh()
      return
    }
    this.focusedKey = row.encoded_key
    // rail-declutter-unread D3：switch 成功 = 聚焦写锚——读锚推进到当前
    // msg_count，徽标清零（无锚点会话自此刻起开始累计未读）。
    writeFocusAnchor(row.encoded_key, row.msg_count)
    if (location.pathname !== '/') navigate('/')
    // fix-webui-qa-defects 4.1（design D3）：switch 只写服务端指针、不发任何
    // 事件——dashboard 的焦点视图此前要等下一个无关会话事件刷新 summary 才
    // 「跳」过来。这里以响应里的 active_session_key 派发窗口级聚焦事件，
    // dashboard 监听后立即走节流刷新（同会话重复派发幂等——刷新收敛到同一
    // 指针，无副作用）。
    window.dispatchEvent(
      new CustomEvent(RAIL_FOCUS_EVENT, {
        detail: { key: resp?.active_session_key ?? row.encoded_key },
      }),
    )
  }

  sessionsFor(id: string) { return this.sessions.filter((r) => r.project_id === id) }

  // ─── New session dialog（workbench-interaction-polish 3.2，design D2）──
  // 项目行「+」是唯一创建入口：打开对话框（agent 必选 + 两级模型 + mode），
  // 确认后 POST /api/sessions 建 0-turn 占位（服务端 set_focus），沿用
  // create 后的就地聚焦链路——create_session 的焦点指针会让 summary 的
  // active_session_key 驱动 composer 进入跟随模式。取消则什么都不发生。
  private openNewSessionDialog(p: Project): void {
    this.newSessionTarget = p
    this.newSessionError = null
  }

  private closeNewSessionDialog(): void {
    this.newSessionTarget = null
    this.newSessionError = null
  }

  private async confirmNewSession(e: CustomEvent<NewSessionDialogConfirm>): Promise<void> {
    const p = this.newSessionTarget
    // （3.4）提交不得静默蒸发：分派条件不成立（项目不可用/请求已在途）时
    // 留在对话框内给 inline 错误——绝不假装创建成功地关闭。
    if (!p) {
      this.newSessionError = '无法创建会话：目标项目不可用，请关闭对话框后重试'
      return
    }
    if (this.creatingSession) return
    this.creatingSession = true
    this.newSessionError = null
    try {
      const created = await api.createSession({
        projectId: p.id,
        agent: e.detail.agent,
        model: e.detail.model,
        mode: e.detail.mode,
      })
      // （round3 3.1）创建即见过：读锚在本浏览器就地立基（0 轮占位）。
      // 创建路径没有 rail 点击（服务端 set_focus 直达焦点），此前锚永远
      // 缺位，而无锚会话按 spec 读作 fully-read——新会话之后的非聚焦新回复
      // 因此永远推不出未读徽章（QA 缺陷 3 的根因）。游标单调：重复创建/
      // 已有更高锚不回退。
      writeFocusAnchor(created.key, 0)
      this.closeNewSessionDialog()
      // workbench-rail-polish 3.1/D2：创建成功后的焦点链三步显式化，不再
      // 借用 onSelect 的 toggle——从已展开的项目行「+」进来会把组误折叠，
      // 新会话行根本不可见。
      // ① 强制展开（非 toggle）：新会话行必须立即可见（随写持久化）。
      this.expanded = { ...this.expanded, [p.path]: true }
      this.writeExpandedStorage()
      // ② 不派发 rail-select：主区项目切换语义不掺进创建路径；新会话行的
      //    「当前」标记由 refresh() 后的 active_session_key 回填（创建请求
      //    服务端已 set_focus）。
      void this.refresh()
      if (location.pathname !== '/') navigate('/')
      // ③ 一次性 composer 对焦请求（COMPOSER_FOCUS_REQUEST，dashboard 接力
      //    到 focusInput）。setTimeout(0)：在 /sessions 上创建时先让路由
      //    切换把工作台挂载出来，监听方才在场。仅创建成功这一个时机派发
      //    ——openSession 等切换路径绝不抢键盘焦点。
      window.setTimeout(
        () => window.dispatchEvent(new CustomEvent(COMPOSER_FOCUS_REQUEST)),
        0,
      )
    } catch (err) {
      // 失败留在对话框内就地呈现——不假装创建成功。
      this.newSessionError = err instanceof Error ? err.message : String(err)
    } finally {
      this.creatingSession = false
    }
  }

  // ─── Remove project（5.1；rail-declutter-unread D5 预检 + 后端强制）──
  // 菜单项路径：事件不再就地 stopPropagation——让它继续冒泡穿过 dropdown
  // 的 menu（handleMenuClick 负责收起菜单），阻断行的职责在 <wa-dropdown>
  // 本体的 @click 上。
  private openRemoveDialog(_e: Event, p: Project) {
    this.removeTarget = p
    this.removeError = null
  }
  private closeRemoveDialog() { this.removeTarget = null; this.removeError = null }
  /** 项目下非归档会话数（rail-declutter-unread D5 弹窗预检的数据源）。 */
  private liveSessionCountFor(id: string): number {
    return this.sessionsFor(id).length
  }
  private async confirmRemoveProject() {
    const p = this.removeTarget
    if (!p || this.removing) return
    this.removing = true
    this.removeError = null
    try {
      await api.projects.remove(p.id)
      this.closeRemoveDialog()
      void this.refresh()
    } catch (err) {
      this.removeError = err instanceof Error ? err.message : String(err)
    } finally {
      this.removing = false
    }
  }

  // ─── Close session（5.2：inactive 直删 / active 需确认）────────────
  @state() private closeTarget: SessionRow | null = null
  @state() private closeError: string | null = null

  // ─── Rename（fix-webui-approval-restore-and-session-identity 5.1）────
  /** 重命名目标（`null` = 对话框关闭）。零轮占位同样可命名。 */
  @state() private renameTarget: SessionRow | null = null
  @state() private renameValue = ''
  @state() private renameError: string | null = null
  @state() private renaming = false

  // ─── Archive（workbench-live-conversation-flow 4.2：唯一出口，确认框
  // 合并 close 语义——终止子进程 + 点名将被丢弃的待执行条数）────────────
  private requestArchive(e: Event, row: SessionRow) {
    void e
    // 归档即关闭（服务端语义）：一律内联确认，不因会话空闲而跳过——
    // 「将丢弃 N 条待执行」的告知不该依赖会话状态。
    this.closeTarget = row
    this.closeError = null
  }
  private closeConfirmDialog() { this.closeTarget = null; this.closeError = null }
  private archiving = false
  private async confirmArchiveSession() {
    const row = this.closeTarget
    if (!row || this.archiving) return
    // 重入护栏：确认按钮的合成 click 可能双发，归档只执行一次。
    this.archiving = true
    try {
      await api.archiveSession(row.encoded_key)
      this.closeConfirmDialog()
      void this.refresh()
    } catch (err) {
      this.closeError = err instanceof Error ? err.message : String(err)
    } finally {
      this.archiving = false
    }
  }

  // ─── Rename（5.1，design D6）：行菜单提交 label；清空 = 回退首条 prompt ─
  private openRenameDialog(e: Event, row: SessionRow) {
    void e
    this.renameTarget = row
    // 预填当前 label（未设置 = 空输入，提交空 = 清空语义）。
    this.renameValue = row.label ?? ''
    this.renameError = null
  }
  private closeRenameDialog() {
    this.renameTarget = null
    this.renameValue = ''
    this.renameError = null
  }
  /**
   * （fix-webui-qa-defects-round5 2.1，design 决策 3）保存时显式从 wa-input
   * 的内部原生 input 取值——QA D3a 实锤：slot 结构下宿主 value 属性与内部
   * 原生 input 可能不同步，依赖宿主属性让输入值在保存链路丢失（保存成静默
   * 清空）。升级后的 wa-input 以 shadowRoot 里的原生 input 为锚；未升级
   * （测试环境）退化为宿主 value，再退组件状态。
   */
  private renameInputValue(): string {
    const host = this.shadowRoot?.querySelector<HTMLInputElement>(
      'wa-input[data-testid="rename-input"]',
    )
    const native = host?.shadowRoot?.querySelector<HTMLInputElement>('input')
    return String(native?.value ?? host?.value ?? this.renameValue ?? '')
  }
  private async confirmRename() {
    const row = this.renameTarget
    if (!row || this.renaming) return
    this.renaming = true
    this.renameError = null
    const label = this.renameInputValue().trim()
    try {
      // 空输入 = 清空（wire 语义单一出处：服务端把空白归一为 None）。
      await api.setSessionLabel(row.encoded_key, label || null)
      this.closeRenameDialog()
      await this.refresh()
    } catch (err) {
      this.renameError = err instanceof Error ? err.message : String(err)
    } finally {
      this.renaming = false
    }
  }

  /**
   * （polish-workbench-walkthrough-ux 2.1）点 History 条目 = 打开只读归档
   * 视图——绝不触发 restore（误触陷阱已拆除）：归档行点击只上报条目，
   * app-shell 接力给 dashboard 渲染只读视图；「恢复」是归档视图里的显式
   * 按钮 + 确认弹窗（confirm 流程与 toast 反馈都在那边）。
   */
  private viewArchivedSession(e: Event, entry: ArchiveEntry) {
    e.stopPropagation()
    this.dispatchEvent(
      new CustomEvent<ArchiveEntry>('rail-archive-view', {
        detail: entry,
        bubbles: true,
        composed: true,
      }),
    )
  }

  // ─── Drag & drop ────────────────────────────────────────────────
  private onDragStart(e: DragEvent, index: number) {
    this.dragIndex = index
    if (e.dataTransfer) { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', String(index)) }
  }
  private onDragOver(e: DragEvent, index: number) {
    if (this.dragIndex === null) return
    e.preventDefault()
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'
    this.dragOverIndex = index
  }
  private onDragLeave(index: number) { if (this.dragOverIndex === index) this.dragOverIndex = null }
  private async onDrop(e: DragEvent, dropIndex: number) {
    e.preventDefault()
    const from = this.dragIndex
    this.dragIndex = null; this.dragOverIndex = null
    if (from === null || from === dropIndex) return
    const next = [...this.projects]; const [moved] = next.splice(from, 1); next.splice(dropIndex, 0, moved)
    this.projects = next
    try {
      const { projects } = await api.projects.reorder(next.map((p) => p.id))
      this.projects = projects
    } catch (err) { this.error = err instanceof Error ? err.message : String(err); void this.refresh() }
  }
  private onDragEnd() { this.dragIndex = null; this.dragOverIndex = null }

  // ─── Add project dialog ─────────────────────────────────────────
  private openAddDialog() {
    this.addDialogOpen = true
    this.addPath = ''
    this.addError = null
    this.addPathScopeHint = null
    // 默认选本机（隐式注册的既有行为）。
    this.addNodeId = ''
    const picker = this.shadowRoot?.querySelector('.folder-picker') as any
    if (picker?.reset) void picker.reset()
  }
  private closeAddDialog() {
    this.addDialogOpen = false
    this.addPath = ''
    this.addError = null
    this.addPathScopeHint = null
  }
  private onFolderSelected(e: CustomEvent) {
    // （round3 4.4）picker 回填值即展示值：分隔符归一后再进输入框，避免
    // 与服务端错误提示里的反斜杠普通形混排。
    this.addPath = normalizeDisplayPath(e.detail.path)
    void this.checkAddPathScope(this.addPath)
  }

  /**
   * 5.2：手填路径的越界预检（fix-webui-qa-defects 7.2）：此前越界路径要等提交
   * 被 400 拒绝才见文案，按钮只按「空路径」禁用——越界被拒时操作者只看到
   * 一个灰按钮、无从得知原因。这里用 browse-dirs（后端同一 safe_path 权威）
   * 预判：越界即输入框旁给出禁用原因并禁用提交；其余失败（不存在等）不
   * 拦截，留给注册接口的点名报错。
   */
  private addPathScopeSeq = 0
  private async checkAddPathScope(path: string) {
    const trimmed = path.trim()
    const seq = ++this.addPathScopeSeq
    if (!trimmed) {
      this.addPathScopeHint = null
      return
    }
    try {
      await api.fsBrowseDirs(trimmed)
      if (seq === this.addPathScopeSeq) this.addPathScopeHint = null
    } catch (e) {
      if (seq !== this.addPathScopeSeq) return
      this.addPathScopeHint = addPathScopeHintFrom(e instanceof Error ? e.message : String(e))
    }
  }
  private async submitAddProject() {
    const path = this.addPath.trim()
    if (!path) { this.addError = '请输入路径'; return }
    // 8.1：节点维度随注册一起走。选定非本机节点时，路径可用性由**该节点**
    // 判定——后端拒绝会点名节点、路径与哪里不对，这里原样呈现。
    const nodeId = this.addNodeId || null
    if (nodeId && !this.nodeOnline(nodeId)) {
      const st = this.nodeStatus(nodeId)
      this.addError = `无法注册到节点 ${nodeId}：${st.cause ?? '节点不可用'}`
      return
    }
    try {
      const p = await api.projects.add(path, nodeId)
      this.closeAddDialog()
      await this.refresh()
      // 降级标记就地呈现（refresh 已清 hint，add 的响应说了算）：核心不可达
      // 时项目仍落栏（本地注册表），但操作者不再直到新建会话才得知。
      this.degradedHint = p.degraded?.cause ? `核心不可达（${p.degraded.cause}），已写入本地注册表` : null
      this.onSelect(p.path)
    } catch (e) {
      // （round3 4.4）「项目已注册」等服务端点名消息里的路径随展示归一，
      // 与输入框里的 picker 回填值同一分隔符。
      this.addError = normalizeDisplayPath(e instanceof Error ? e.message : String(e))
    }
  }

  private countsFor(id: string): { count: number; waiting: boolean } {
    let count = 0; let waiting = false
    for (const r of this.sessions) {
      if (r.project_id !== id) continue
      count += 1
      // 8.4 + polish-workbench-walkthrough-ux 3.6：橙点只标「在等操作员
      // 决定」（悬空审批 > 0 / waiting）——queued/starting/failed 的占位与
      // 排队不属于「需介入」，全新占位会话不再误亮橙点。
      if (r.status_slug === 'waiting' || (r.remote?.parked_approvals ?? 0) > 0) {
        waiting = true
      }
    }
    return { count, waiting }
  }

  /**
   * 8.4：等待操作员决定（悬空审批 > 0）的会话。它们单独成组——「在等人」与
   * 「在干活」必须是两个可分辨的集合，把等待埋在项目分组里就等于只有点开
   * 才发现。
   */
  waitingSessions(): SessionRow[] {
    return this.sessions.filter((r) => (r.remote?.parked_approvals ?? 0) > 0)
  }

  // ─── Renderers ──────────────────────────────────────────────────

  private renderSessionRow(row: SessionRow) {
    const fullLabel = fullSessionLabel(row)
    const label = truncateName(fullLabel)
    // 当前标记由焦点指针驱动（3.2）：不再比较 location.pathname。
    const current = this.focusedKey === row.encoded_key
    // rail-declutter-unread 2.3：未读徽标 = msg_count − 共享读锚；0 或负数
    // 不显示，99+ 封顶。
    const unread = unreadCount(row.encoded_key, row.msg_count)
    const badge = unread > UNREAD_BADGE_CAP ? `${UNREAD_BADGE_CAP}+` : String(unread)
    // 8.4：悬空审批 > 0 = 在等人，不是在跑（状态词由后端投影为 waiting，
    // 这里再按 remote 兜一层，老报文/直接 mock 的 remote 也能正确标）。
    const remote = row.remote ?? null
    const waiting = (remote?.parked_approvals ?? 0) > 0 || row.status_slug === 'waiting'
    // 8.5：会话标注所属节点；节点不可用时把成因写在 title 上。
    const nodeId = remote?.node_id ?? null
    const nodeLabel = nodeId ?? (row.project_id ? LOCAL_NODE : null)
    const nodeOffline = remote != null && remote.node_status !== 'online'
    const nodeTitle = nodeOffline
      ? `节点 ${nodeId}：${remote?.node_cause ?? remote?.node_status ?? '不可用'}`
      : nodeLabel
        ? `执行节点 ${nodeLabel}`
        : ''
    return html`
      <li
        class="session-item ${current ? 'current' : ''} ${waiting ? 'waiting' : ''} ${nodeOffline ? 'node-offline' : ''} ${unread > 0 ? 'unread' : ''}"
        title=${fullLabel}
        aria-current=${current ? 'true' : 'false'}
        @click=${() => this.openSession(row)}
      >
        <span class="session-dot" data-status=${waiting ? 'waiting' : row.status_slug} aria-hidden="true"></span>
        <span class="session-name">${label}</span>
        ${unread > 0 ? html`<span class="unread-badge" data-testid="session-unread" title="${unread} 条未读回复">${badge}</span>` : nothing}
        ${nodeLabel ? html`<span class="session-node" data-testid="session-node" title=${nodeTitle}>${nodeOffline ? '⚠ ' : ''}${nodeLabel}</span>` : nothing}
        ${waiting ? html`<span class="wait-badge" data-testid="session-waiting" title="等待操作员决定（悬空审批 ${remote?.parked_approvals ?? 0}）">等待${(remote?.parked_approvals ?? 0) > 0 ? ` ${remote!.parked_approvals}` : ''}</span>` : nothing}
        <!-- 菜单：触发钮的 click 必须能冒泡到 dropdown 的 trigger slot（打开
             菜单的监听在那里）；阻断行级 click 的位置在 <wa-dropdown> 本体。 -->
        <wa-dropdown placement="bottom-end" @click=${(e: Event) => e.stopPropagation()}>
          <button
            slot="trigger"
            class="row-action"
            title="Session actions"
            aria-label="Session actions for ${fullLabel}"
            aria-haspopup="menu"
          >${icon('more', 12)}</button>
          <!-- （workbench-live-conversation-flow 4.2）归档是唯一生命周期
               出口，自带 close 语义（终止子进程 + 丢弃待执行，确认框点名）。 -->
          <!-- （5.1，design D6）重命名：设置 label 后行名与对话框同源优先
               显示 label；清空回退首条 prompt 预览。 -->
          <wa-dropdown-item value="rename" @click=${(e: Event) => this.openRenameDialog(e, row)}>重命名</wa-dropdown-item>
          <wa-dropdown-item value="archive" variant="danger" @click=${(e: Event) => this.requestArchive(e, row)}>归档</wa-dropdown-item>
        </wa-dropdown>
      </li>`
  }

  private renderArchivedSessionRow(a: ArchiveEntry) {
    return html`
      <li
        class="session-item archived"
        title=${a.label}
        data-testid="archived-row"
        @click=${(e: Event) => this.viewArchivedSession(e, a)}
      >
        <span class="session-dot done" aria-hidden="true"></span>
        <span class="session-name">${a.label}</span>
        <!-- （fix-webui-qa-defects-round4 3.2）basename 切分同时接受 \ 与 /：
             Windows 归档路径存的是反斜杠普通形，此前 split('/') 不切、整条
             路径原样进 .archive-meta，撑出横向滚动。 -->
        <span class="archive-meta">${a.project_path.split(/[\\/]/).filter(Boolean).pop() ?? ''}</span>
      </li>`
  }

  private renderRow(p: Project, index: number) {
    const info = this.branchByPath[p.id]
    const accessible = info ? info.accessible : true
    const { count, waiting } = this.countsFor(p.id)
    const isActive = this.activePath === p.path
    // （4.2）持久化记录优先、聚焦所在项目无记录时缺省展开。
    const isExpanded = this.isProjectExpanded(p.path)
    const dragging = this.dragIndex === index
    const dragOver = this.dragOverIndex === index && this.dragIndex !== null && this.dragIndex !== index
    const projectSessions = this.sessionsFor(p.id)
    // 8.2/8.5：项目标注所属节点；节点不可用时阻止新建（composer/rail 同一门禁）
    // 并把成因写出来，而不是等提交失败才说。
    const st = this.nodeStatus(p.node_id)
    const nodeLabel = p.node_id || LOCAL_NODE
    const nodeOk = st.status === 'online'
    const nodeTitle = nodeOk ? `执行节点 ${nodeLabel}` : `节点 ${nodeLabel}：${st.cause ?? st.status}`
    return html`
      <li>
        <div class=${['row', isActive ? 'active' : '', accessible ? '' : 'unreachable', nodeOk ? '' : 'node-offline', dragging ? 'dragging' : '', dragOver ? 'drag-over' : ''].filter(Boolean).join(' ')} draggable="true" aria-current=${isActive ? 'true' : 'false'} aria-expanded=${isExpanded ? 'true' : 'false'} @click=${() => this.onSelect(p.path)} @dragstart=${(e: DragEvent) => this.onDragStart(e, index)} @dragover=${(e: DragEvent) => this.onDragOver(e, index)} @dragleave=${() => this.onDragLeave(index)} @drop=${(e: DragEvent) => this.onDrop(e, index)} @dragend=${() => this.onDragEnd()}>
          <span class="name">
            <span>${p.name}</span>
            <span class="node-chip" data-testid="project-node" data-node-status=${st.status} title=${nodeTitle}>${nodeOk ? '' : '⚠ '}${nodeLabel}</span>
            ${waiting ? html`<span class="wait-dot" title="需要操作员介入" aria-label="需介入"></span>` : nothing}
          </span>
          <span class="meta">${count > 0 ? html`<span class="count">${count}</span>` : nothing}</span>
          <span class="row-actions">
            <!-- rail-declutter-unread 3.1：「…」在前、「+」在后（顺序固定）。
                 移除动作收进「…」菜单（现阶段仅此一项，留扩展位）；分支名
                 不再显示，可达性探测保留（loadBranch/删除线告警不变）。
                 触发钮的 click 必须能冒泡到 dropdown 的 trigger slot（打开
                 菜单的监听在那里）；阻断行级 click 的位置在 <wa-dropdown>。 -->
            <wa-dropdown placement="bottom-end" @click=${(e: Event) => e.stopPropagation()}>
              <button
                slot="trigger"
                class="row-action"
                title="Project actions"
                aria-label="Project actions for ${p.name}"
                aria-haspopup="menu"
              >${icon('more', 12)}</button>
              <wa-dropdown-item value="remove" @click=${(e: Event) => this.openRemoveDialog(e, p)}>移除项目</wa-dropdown-item>
            </wa-dropdown>
            <button
              class="row-action"
              title=${nodeOk ? `New session in ${p.name}` : `无法新建会话：${st.cause ?? `节点 ${nodeLabel} 不可用`}`}
              aria-label="New session in ${p.name}"
              ?disabled=${!nodeOk}
              @click=${(e: Event) => {
                e.stopPropagation()
                this.openNewSessionDialog(p)
              }}
            >${icon('add', 12)}</button>
          </span>
        </div>
        ${nodeOk ? nothing : html`<div class="node-cause" data-testid="project-node-cause">节点 ${nodeLabel} 不可用：${st.cause ?? st.status}</div>`}
        ${isExpanded ? (projectSessions.length > 0 ? html`<ul class="sessions">${projectSessions.map((r) => this.renderSessionRow(r))}</ul>` : html`<div class="empty">该项目暂无会话</div>`) : nothing}
      </li>`
  }

  /**
   * 8.4：等待操作员决定的会话单独成组（跨项目）。它们本来也会出现在各自
   * 项目的展开列表里，但「有人在等你」不该要求操作员逐个展开才发现。
   */
  private renderWaiting() {
    const waiting = this.waitingSessions()
    if (waiting.length === 0) return nothing
    return html`
      <div class="group-section waiting-group">
        <button type="button" class="group-head" aria-expanded=${this.waitingOpen ? 'true' : 'false'} @click=${() => (this.waitingOpen = !this.waitingOpen)}>
          <span class="chevron ${this.waitingOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>Waiting on you</span><span class="group-count">${waiting.length}</span>
        </button>
        ${this.waitingOpen ? html`<ul class="sessions">${waiting.map((r) => this.renderSessionRow(r))}</ul>` : nothing}
      </div>`
  }

  private renderHistory() {
    // rail-declutter-unread D7：History 按归档时间倒序（新的在前）。前端
    // 排序，/api/archive 保持插入序返回（wire 契约不变）。
    const archived = [...this.archivedSessions].sort((a, b) => b.archived_at - a.archived_at)
    if (archived.length === 0) return nothing
    return html`
      <div class="group-section">
        <button type="button" class="group-head" data-testid="history-group-head" aria-expanded=${this.historyOpen ? 'true' : 'false'} @click=${() => (this.historyOpen = !this.historyOpen)}>
          <span class="chevron ${this.historyOpen ? 'open' : ''}" aria-hidden="true">▶</span><span>History</span><span class="group-count">${archived.length}</span>
        </button>
        ${this.historyOpen ? html`<ul class="sessions">${archived.map((a) => this.renderArchivedSessionRow(a))}</ul>` : nothing}
      </div>`
  }

  render() {
    // workbench-turn-queue 7.4：关闭确认对话框点名将被丢弃的待执行条数。
    const closePendingCount = this.closeTarget?.pending_count ?? 0
    return html`
      <div class="section-label">
        <span>Projects</span>
        <button class="add-btn" aria-label="Add project" title="添加项目" @click=${this.openAddDialog}>${icon('add', 14)}</button>
      </div>
      ${this.error ? html`<div class="error">${this.error} <button class="retry-btn" @click=${() => void this.refresh()}>重试</button></div>` : nothing}
      ${this.degradedHint ? html`<div class="degraded-hint" role="status" data-testid="project-degraded-hint">${this.degradedHint}</div>` : nothing}
      ${this.projects.length === 0 ? html`<div class="empty">尚未注册项目</div>` : html`<ul>${this.projects.map((p, i) => this.renderRow(p, i))}</ul>`}
      ${this.renderWaiting()}
      ${this.renderHistory()}

      ${this.addDialogOpen ? html`
      <wa-dialog label="Add project" style="--width: 480px;" .open=${true} @wa-hide=${guardedHide(() => this.closeAddDialog())}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">
          <p style="font-size:0.85rem;color:var(--sebas-text);margin:0;">Choose a directory to add as a project:</p>
          <sebas-folder-picker class="folder-picker" @folder-selected=${this.onFolderSelected}></sebas-folder-picker>
          <p style="font-size:0.8rem;color:var(--sebas-text-faint);margin:0;text-align:center;">or</p>
          <!-- Web Awesome 3.x 派发标准 input 事件（不派发 wa-input），手动路径才能联动启用提交按钮。
               fix-webui-qa-defects 7.2：手填路径即时预检越界（禁用不再静默）。
               fix-webui-approval-restore-and-session-identity 5.2：本行行尾曾有
               一个游离引号（.value 绑定后跟一个未配对的引号再接标签收尾）——
               属性未加引号，该引号被并进属性值，把 @input 的 EventPart 降级成
               普通属性 part，listener 永不挂接（历轮「手填路径失灵」的根因）；
               同时把引号灌进 .value 值。引号已除，@input 绑定恢复。 -->
          <wa-input label="Project path" placeholder="/absolute/path/to/repo" .value=${this.addPath} @input=${(e: any) => { this.addPath = e.target.value; void this.checkAddPathScope(this.addPath) }}>
            <wa-icon slot="start" name="folder" aria-hidden="true"></wa-icon>
          </wa-input>
          <!-- 8.1：节点维度。空值 = 本机隐式注册（既有行为）；选远端时路径由
               那台节点判定。远端注册表不可得时如实说明，不假装没有远端节点。 -->
          <wa-select
            label="Execution node"
            data-testid="add-node-select"
            value=${this.addNodeId}
            @change=${(e: any) => (this.addNodeId = e.target.value ?? '')}
          >
            <wa-option value="">local（本机，隐式）</wa-option>
            ${this.nodes
              .filter((n) => !n.local && n.id !== LOCAL_NODE)
              .map((n) => html`<wa-option value=${n.id} ?disabled=${n.status !== 'online'}>${n.status === 'online' ? n.id : `${n.id}（${n.status}）`}</wa-option>`)}
          </wa-select>
          ${this.remoteNodesAvailable
            ? nothing
            : html`<p style="font-size:0.75rem;color:var(--sebas-text-faint);margin:0;">远端节点状态不可得${this.nodesCause ? `：${this.nodesCause}` : ''}</p>`}
          ${this.addError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;" data-testid="add-project-error">${this.addError}</div>` : nothing}
          <!-- fix-webui-qa-defects 7.2：越界禁用原因在输入框旁可见，不再静默。 -->
          ${this.addPathScopeHint ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;" data-testid="add-project-scope-hint">${this.addPathScopeHint}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="brand" @click=${() => void this.submitAddProject()} ?disabled=${!this.addPath.trim() || this.addPathScopeHint !== null}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeAddDialog()}>Cancel</wa-button>
      </wa-dialog>` : nothing}

      ${this.removeTarget !== null ? html`
      <wa-dialog label="Remove project" style="--width: 440px;" .open=${true} @wa-hide=${guardedHide(() => this.closeRemoveDialog())}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            移除项目 <b>${this.removeTarget?.name ?? ''}</b>？
          </p>
          ${this.removeTarget !== null && this.liveSessionCountFor(this.removeTarget.id) > 0
            ? html`<p
                style="font-size:0.8rem;color:var(--sebas-status-failed);margin:0;"
                data-testid="remove-blocked"
              >
                该项目下仍有 <b>${this.liveSessionCountFor(this.removeTarget.id)}</b>
                个未归档会话——请先在会话行的 <b>…</b> 菜单里归档或关闭它们（共
                ${this.liveSessionCountFor(this.removeTarget.id)} 个），再移除项目。
              </p>`
            : html`<p style="font-size:0.8rem;color:var(--sebas-text-dim);margin:0;">
                此操作只解除注册，可重新添加。
              </p>`}
          ${this.removeError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.removeError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="danger" ?loading=${this.removing} @click=${() => void this.confirmRemoveProject()}>移除</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeRemoveDialog()}>取消</wa-button>
      </wa-dialog>` : nothing}

      ${this.closeTarget !== null ? html`
      <wa-dialog label="归档会话" style="--width: 440px;" .open=${true} @wa-hide=${guardedHide(() => this.closeConfirmDialog())}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
            归档会话 <b>${this.closeTarget ? truncateName(fullSessionLabel(this.closeTarget)) : ''}</b>？
          </p>
          <p style="font-size:0.8rem;color:var(--sebas-text-dim);margin:0;">
            归档会终止 agent 子进程并把会话移入 History（只读，可恢复），不可撤销。
          </p>
          ${closePendingCount > 0
            ? html`<p
                style="font-size:0.8rem;color:var(--sebas-status-failed);margin:0;"
                data-testid="close-discards-pending"
              >
                将丢弃 <b>${closePendingCount}</b> 条待执行消息，它们不会被执行。
              </p>`
            : nothing}
          ${this.closeError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;">${this.closeError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="danger" @click=${() => void this.confirmArchiveSession()}>归档</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeConfirmDialog()}>取消</wa-button>
      </wa-dialog>` : nothing}

      <!-- （5.1）重命名对话框：label 是操作者自由输入，零轮占位同样可命名；
           空输入 = 清空。关闭即整棵移出 ARIA 树。 -->
      ${this.renameTarget !== null ? html`
      <wa-dialog label="重命名会话" style="--width: 440px;" .open=${true} @wa-hide=${guardedHide(() => this.closeRenameDialog())}>
        <div class="wa-stack" style="gap:var(--sebas-space-3);">
          <wa-input
            data-testid="rename-input"
            label="会话名称"
            placeholder="留空则回退到首条消息预览"
            .value=${this.renameValue}
            @input=${(e: Event) => (this.renameValue = (e.target as HTMLInputElement).value)}
          ></wa-input>
          ${this.renameError ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;" data-testid="rename-error">${this.renameError}</div>` : nothing}
        </div>
        <wa-button slot="footer" variant="brand" ?loading=${this.renaming} @click=${() => void this.confirmRename()}>保存</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.closeRenameDialog()}>取消</wa-button>
      </wa-dialog>` : nothing}

      <!-- 创建会话对话框（workbench-interaction-polish D2）：唯一可选 agent
           的地方；项目行「+」打开，确认后由 rail 落 POST /api/sessions。
           （5.3）关闭即整棵移出 ARIA 树，不留残影。 -->
      ${this.newSessionTarget !== null ? html`
      <sebas-new-session-dialog
        data-testid="new-session-dialog"
        .open=${true}
        .projectId=${this.newSessionTarget?.id ?? null}
        .projectName=${this.newSessionTarget?.name ?? null}
        .defaultAgent=${this.newSessionTarget?.default_agent ?? null}
        .error=${this.newSessionError}
        .busy=${this.creatingSession}
        @dialog-confirm=${(e: CustomEvent<NewSessionDialogConfirm>) =>
          void this.confirmNewSession(e)}
        @dialog-cancel=${() => this.closeNewSessionDialog()}
      ></sebas-new-session-dialog>` : nothing}`
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-project-rail': SebasProjectRail
  }
}