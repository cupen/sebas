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
 * is session-scoped — creation mode at `/` with nothing focused, follow-up
 * mode on a session detail.
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
  readonly newSessionChip: Locator
  readonly backendSelect: Locator
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
    this.newSessionChip = page.locator('sebas-workbench-composer .mode-chip')
    this.backendSelect = page.locator('sebas-workbench-composer wa-select.backend-select')
    this.reachabilityWarning = page.locator(
      'sebas-workbench-composer .callout-warning[role="status"]',
    )
  }

  /** Type a prompt and submit with Enter (the composer's keyboard path). */
  async sendPrompt(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }

  /** The provider label chip ("anthropic" in the sandbox) or its honest placeholder. */
  providerLabel(): Locator {
    return this.composer.locator('.composer-bottom .left-tools .label').first()
  }

  /** The "native · built-in kernel (unavailable: …)" option, disabled. */
  nativeOption(): Locator {
    return this.backendSelect.locator('wa-option[value="native"]')
  }

  /** The model dropdown — D4 honest absence: MUST NOT render without model options. */
  modelSelect(): Locator {
    return this.composer.locator('wa-select.backend-select.model-select')
  }
}

/**
 * Session detail view (`sebas-session-detail`): status head, original
 * prompt quote, transcript, pinned composer, close dialog.
 */
export class SessionDetailPage {
  readonly page: Page
  readonly host: Locator
  readonly head: Locator
  readonly statusBadge: Locator
  readonly chatId: Locator
  readonly promptQuote: Locator
  readonly transcript: Locator
  readonly emptyTranscript: Locator
  readonly composerTextarea: Locator
  readonly sendButton: Locator
  readonly closeButton: Locator
  readonly closeDialog: Locator
  readonly closeDialogConfirm: Locator
  readonly errorCallout: Locator
  readonly backToWorkbench: Locator
  readonly modelPick: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-session-detail')
    this.head = page.locator('sebas-session-detail .head')
    this.statusBadge = page.locator('sebas-session-detail .head sebas-status-badge')
    this.chatId = page.locator('sebas-session-detail .head .ident .chat')
    this.promptQuote = page.locator('sebas-session-detail blockquote.prompt')
    this.transcript = page.locator('sebas-session-detail section.transcript')
    this.emptyTranscript = page.locator('sebas-session-detail section.transcript .empty')
    this.composerTextarea = page.locator('sebas-session-detail .composer wa-textarea textarea')
    this.sendButton = page.locator('sebas-session-detail .composer .send-button')
    this.closeButton = page.locator('sebas-session-detail .head .actions wa-button')
    this.closeDialog = page.locator('sebas-session-detail wa-dialog[label="Close session"]')
    this.closeDialogConfirm = page
      .locator('sebas-session-detail wa-dialog[label="Close session"] wa-button')
      .filter({ hasText: 'Close session' })
    this.errorCallout = page
      .locator('sebas-session-detail .callout-error[role="alert"]')
      .first()
    this.backToWorkbench = page.locator('sebas-session-detail a', { hasText: '← Back to workbench' })
    this.modelPick = page.locator('sebas-session-detail .head .model-pick')
  }

  /** Status badge's current slug (`data`-driven attribute on the element). */
  async statusSlug(): Promise<string> {
    return (await this.statusBadge.getAttribute('slug')) ?? ''
  }

  /** All transcript bubble bodies as text. */
  bubbles(): Locator {
    return this.page.locator('sebas-session-detail sebas-transcript-view .turn-block .body')
  }

  /** The transcript turn block containing `text` (this page's own transcript). */
  turnWith(text: string): Locator {
    return this.page.locator('sebas-session-detail sebas-transcript-view .turn-block', {
      hasText: text,
    })
  }

  /**
   * Send a follow-up message through the session-detail composer (in
   * follow-up mode the composer targets this session). Submits with Enter.
   */
  async sendFollowUp(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }
}
