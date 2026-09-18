/**
 * Page objects: the app shell (nav/outlet/sidebar) and the workbench main
 * area (project header, turn-stream, composer). One object per custom
 * element, selectors mirroring the frontend's class/aria hooks.
 */
import { expect, type Locator, type Page } from '@playwright/test'

/** The app shell: auth state, sidebar footer, outlet routing. */
export class AppShell {
  readonly page: Page
  readonly app: Locator
  readonly brand: Locator
  readonly outlet: Locator

  constructor(page: Page) {
    this.page = page
    this.app = page.locator('sebas-app')
    this.brand = page.locator('sebas-app nav .brand .name')
    this.outlet = page.locator('sebas-app main .outlet')
  }

  async logout(username: string): Promise<void> {
    await page$logout(this.page, username)
  }
}

/** Sidebar footer logout button: `退出 (${username})`. */
async function page$logout(page: Page, username: string): Promise<void> {
  await page
    .locator('sebas-app .sidebar-footer .settings-btn', { hasText: `退出 (${username})` })
    .click()
}

/**
 * The workbench main area (`sebas-dashboard`): project header, turn-stream
 * stage (empty state / focused transcript), and the composer. The composer
 * is pure follow-up mode（workbench-interaction-polish：创建唯一入口是 rail
 * 的创建对话框，composer 无任何创建/设置控件）— it renders only when a
 * session is focused; with no focus it shows the rail-creation hint.
 */
export class Workbench {
  readonly page: Page
  readonly host: Locator
  readonly projectHeader: Locator
  readonly noProjectSelected: Locator
  readonly emptyStream: Locator
  readonly composer: Locator
  readonly composerTextarea: Locator
  readonly sendButton: Locator
  readonly submitControl: Locator
  readonly modelChip: Locator
  readonly modelMenu: Locator
  readonly noFocusHint: Locator
  readonly reachabilityWarning: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-dashboard')
    this.projectHeader = page.locator('sebas-dashboard .project-header')
    this.noProjectSelected = page.locator('sebas-dashboard .project-header .path.muted')
    this.emptyStream = page.locator('sebas-dashboard .empty-stream')
    this.composer = page.locator('sebas-workbench-composer')
    // Web Awesome wa-textarea wraps a native <textarea> in its open shadow
    // root — the piercing CSS engine reaches it directly.
    this.composerTextarea = page.locator('sebas-workbench-composer wa-textarea textarea')
    this.sendButton = page.locator('sebas-workbench-composer .send-button')
    // D4 状态机控件（disabled | send | sending | stop | queued）。
    this.submitControl = page.locator('sebas-workbench-composer [data-testid="submit-control"]')
    // D3 模型芯片与两级分组菜单。
    this.modelChip = page.locator('sebas-workbench-composer [data-testid="model-chip"]')
    this.modelMenu = page.locator('sebas-workbench-composer [data-testid="model-menu"]')
    // 无聚焦会话时的显式提示（指向 rail 创建入口）。
    this.noFocusHint = page.locator('sebas-workbench-composer [data-testid="composer-no-focus"]')
    this.reachabilityWarning = page.locator(
      'sebas-workbench-composer .callout-warning[role="status"]',
    )
  }

  /**
   * Type a prompt and submit with Enter (the composer's keyboard path).
   * Follow-up mode only — creation lives in the rail's creation dialog.
   */
  async sendPrompt(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }

  /** The submit control's current state attribute (design D4). */
  submitState(): Promise<string | null> {
    return this.submitControl.getAttribute('data-state')
  }
}

/**
 * The focused session ON THE WORKBENCH (workbench-conversation-view 3.3/3.4:
 * the retired `sebas-session-detail` view is gone - `/sessions/:key` deep
 * links and rail switches render the same `sebas-dashboard` focused). One
 * object for the session head（display-only since workbench-live-conversation-
 * flow 4.2：badge/chat id/mode 标签——close/archive 收进 rail 行菜单、模型
 * 芯片归 composer 底沿）, the conversation stream, and the follow-up composer.
 */
export class FocusedSession {
  readonly page: Page
  readonly host: Locator
  readonly sessionHead: Locator
  readonly statusBadge: Locator
  readonly chatId: Locator
  readonly transcript: Locator
  readonly emptyConversation: Locator
  readonly composerTextarea: Locator
  readonly sendButton: Locator
  readonly unavailableNote: Locator
  readonly modelPick: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-dashboard')
    this.sessionHead = page.locator('sebas-dashboard .session-head')
    // （session-parallel-liveness-and-unread-polish 3.6/D6b）会话状态只挂 rail
    // 行首彩色圆点一处：session-head 的 status-badge 与状态边框已下线，焦点
    // 会话的状态面改由 rail 当前行（`li.session-item.current`）的圆点承载。
    this.statusBadge = page.locator('sebas-project-rail li.session-item.current .session-dot')
    this.chatId = page.locator('sebas-dashboard .session-head .ident .chat')
    this.transcript = page.locator('sebas-dashboard .turn-stream-area')
    this.emptyConversation = page
      .locator('sebas-dashboard .turn-stream-area .empty-stream')
      .filter({ hasText: 'Nothing yet' })
    this.unavailableNote = page
      .locator('sebas-dashboard .turn-stream-area .empty-stream')
      .filter({ hasText: 'Session unavailable' })
    // The composer is the dashboard's own workbench composer (follow-up mode).
    this.composerTextarea = page.locator('sebas-workbench-composer wa-textarea textarea')
    this.sendButton = page.locator('sebas-workbench-composer .send-button')
    // 模型选择器（4.2：头部去交互化后归 composer 底沿的芯片）。
    this.modelPick = page.locator('sebas-workbench-composer [data-testid="model-chip"]')
  }

  /** 状态圆点的当前 slug（rail 行 `data-status` = 七词相位）。 */
  async statusSlug(): Promise<string> {
    return (await this.statusBadge.getAttribute('data-status')) ?? ''
  }

  /**
   * 焦点会话的相位断言（D6b：会话状态只挂 rail 行首彩色圆点一处）。会话行落在
   * 项目行的折叠体里——先展开项目行，否则行不在 DOM。展开是幂等的（行本身即
   * 展开开关，重复点会收起，故先读 `aria-expanded`）。
   *
   * ⚠ 展开 = 点项目行 = 同时**选中**该项目（工作台随 rail-select 重绘一次）。
   * 断言之后再读工作台 DOM（如数气泡）的用例，要先等它落回稳定值。
   */
  async expectStatus(slug: string, timeout = 15_000): Promise<void> {
    await this.expandAllProjects()
    await expect(this.statusBadge).toHaveAttribute('data-status', slug, { timeout })
  }

  /** 展开 rail 里所有收起（`aria-expanded=false`）的项目行。 */
  private async expandAllProjects(): Promise<void> {
    const collapsed = this.page.locator(
      'sebas-project-rail li > div.row[aria-expanded="false"]',
    )
    for (let guard = 0; guard < 10; guard++) {
      if ((await collapsed.count()) === 0) return
      await collapsed.first().click()
    }
  }

  /**
   * All conversation bubble bodies (both sides) as text. 折叠体不算气泡：
   * `.body.fold-body`（tool/thinking 过程折叠）与 `.body.item-body`（二级条目）
   * 只在展开时渲染，混进来会让「气泡数」随折叠开合漂移（曾表现为 flaky）。
   */
  bubbles(): Locator {
    return this.page.locator(
      'sebas-dashboard sebas-transcript-view .turn-block .body:not(.fold-body):not(.item-body)',
    )
  }

  /** The conversation turn block containing `text` (either side). */
  turnWith(text: string): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view .turn-block', {
      hasText: text,
    })
  }

  /** The operator's own turn bubble carrying `text` (2.3: both sides render). */
  userTurn(text: string): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view .turn-block.is-user', {
      hasText: text,
    })
  }

  /** An agent turn bubble carrying `text`. */
  agentTurn(text: string): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view .turn-block.is-assistant', {
      hasText: text,
    })
  }

  /**
   * The agent turn's single process fold（workbench-agent-identity-and-
   * process-folds 2.1/2.2: one collapsed fold per turn, link label
   * `process + 尾条目 title + 计数`）. 折叠体是懒渲染的（收起时不在 DOM）。
   */
  processFold(): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view div.process-fold')
  }

  /**
   * Second-level per-entry folds inside the process fold（2.2: each thinking
   * segment / tool invocation, collapsed by default, titled by the entry's
   * structured `title`）.
   */
  processItems(): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view div.process-item')
  }

  /**
   * 展开转写里所有收起的折叠（外层 process 折叠 + 二级条目）。折叠体**懒渲染**
   * ——不展开就不在 DOM 里，所以任何「工具/结果文本可见」的断言都必须先展开。
   */
  async expandAllFolds(): Promise<void> {
    const collapsed = this.page.locator(
      'sebas-dashboard sebas-transcript-view button.fold-link[aria-expanded="false"]',
    )
    // 逐次点击（展开会往 DOM 里塞新的折叠），带上限防死循环。
    for (let guard = 0; guard < 20; guard++) {
      if ((await collapsed.count()) === 0) return
      await collapsed.first().click()
    }
  }

  /**
   * 折叠体内文本的断言（工具结果等）：定稿重分组会把条目折回收起态，且折叠
   * 体懒渲染——一次展开不足以把它留在 DOM，所以「展开 → 数一下 → 不够再
   * 展开」轮询到出现为止。
   */
  async expectFoldedText(text: string, timeout = 20_000): Promise<void> {
    await expect
      .poll(
        async () => {
          await this.expandAllFolds()
          return this.turnWith(text).count()
        },
        { timeout },
      )
      .toBeGreaterThan(0)
  }

  /**
   * Send a follow-up through the dashboard composer (follow-up mode targets
   * the focused session). Submits with Enter.
   */
  async sendFollowUp(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }
}
