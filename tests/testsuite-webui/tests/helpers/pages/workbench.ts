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
 * object for the session head (badge/chat id/model pick/close/archive), the
 * conversation stream, and the follow-up composer.
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
  readonly closeButton: Locator
  readonly archiveButton: Locator
  readonly closeDialog: Locator
  readonly closeDialogConfirm: Locator
  readonly unavailableNote: Locator
  readonly modelPick: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-dashboard')
    this.sessionHead = page.locator('sebas-dashboard .session-head')
    this.statusBadge = page.locator('sebas-dashboard .session-head sebas-status-badge')
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
    this.closeButton = page
      .locator('sebas-dashboard .session-head wa-button')
      .filter({ hasText: 'Close' })
    this.archiveButton = page
      .locator('sebas-dashboard .session-head wa-button')
      .filter({ hasText: 'Archive' })
    this.closeDialog = page.locator('sebas-dashboard wa-dialog[label="Close session"]')
    this.closeDialogConfirm = page
      .locator('sebas-dashboard wa-dialog[label="Close session"] wa-button')
      .filter({ hasText: 'Close session' })
    this.modelPick = page.locator('sebas-dashboard .session-head .model-pick')
  }

  /** Status badge's current slug (`data`-driven attribute on the element). */
  async statusSlug(): Promise<string> {
    return (await this.statusBadge.getAttribute('slug')) ?? ''
  }

  /** All conversation bubble bodies (both sides) as text. */
  bubbles(): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view .turn-block .body')
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

  /** The turn's expandable tool group (2.2: "used N tools" group). */
  toolGroup(): Locator {
    return this.page.locator('sebas-dashboard sebas-transcript-view details.tools-fold')
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
