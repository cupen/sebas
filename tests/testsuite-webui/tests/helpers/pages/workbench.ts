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

  /**
   * The session model dropdown (follow-up mode) — D4 honest absence: MUST
   * NOT render when the agent exposes no model option. In creation mode the
   * two-level catalog offers provider + model selects instead (4.3), so the
   * provider select is excluded here.
   */
  modelSelect(): Locator {
    return this.composer.locator('wa-select.backend-select.model-select:not(.provider-select)')
  }

  /** Creation-mode catalog level 1: the provider select（4.3）. */
  providerSelect(): Locator {
    return this.composer.locator('wa-select.provider-select')
  }

  /** Creation-mode catalog level 2: the model select（4.3）. */
  catalogModelSelect(): Locator {
    return this.composer.locator('wa-select[aria-label="Model"]')
  }

  /** Creation-mode honest degradation: the explicit unavailability note（4.4）. */
  catalogUnavailable(): Locator {
    return this.composer.locator('[data-testid="catalog-unavailable"]')
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
