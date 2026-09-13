/**
 * Page objects: the sidebar project rail (`sebas-project-rail`), the
 * transcript (`sebas-transcript-view`), review cards
 * (`sebas-review-cards`), the sessions list (`sebas-sessions`), and the
 * settings modal (`sebas-settings-modal`).
 */
import { expect, type Locator, type Page } from '@playwright/test'

/** Sidebar project tree: projects, per-project sessions, Waiting/History groups (rail-declutter-unread: Inbox 分组已移除，行操作收敛为 … 菜单). */
export class ProjectRail {
  readonly page: Page
  readonly host: Locator
  readonly addButton: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-project-rail')
    this.addButton = page.locator('sebas-project-rail .section-label .add-btn')
  }

  /** A project row by displayed name. */
  projectRow(name: string): Locator {
    return this.host.locator('.row', { hasText: name })
  }

  /** Expand a project row to reveal its sessions (204a901 起行本身即展开开关). */
  async expandProject(name: string): Promise<void> {
    await this.projectRow(name).click()
  }


  /** Expand the History group (archived sessions; hidden when empty). */
  async expandHistory(): Promise<void> {
    await this.host.locator('.group-head', { hasText: 'History' }).click()
  }

  /**
   * Session row by its display label (the `session_id_short` text the rail
   * renders; backend rows carry no chat_id).
   */
  sessionItem(label: string): Locator {
    return this.host.locator('li.session-item', { hasText: label }).first()
  }

  /**
   * Open a session row's overflow (…) menu (rail-declutter-unread 3.2).
   * The trigger is hover-revealed, so hover first (Playwright treats
   * opacity-0 as hidden), then click it and wait for the items.
   */
  async openSessionMenu(rowLabel: string): Promise<Locator> {
    const row = this.host
      .locator('li.session-item:not(.archived)', { hasText: rowLabel })
      .first()
    await row.hover()
    await row.locator('wa-dropdown button[title="Session actions"]').click()
    const menu = row.locator('wa-dropdown-item[value="archive"]')
    await expect(menu).toBeVisible()
    return menu
  }

  /**
   * Archive a session row (by its displayed label — since
   * rail-declutter-unread the first prompt's preview). Goes through the
   * row's … menu.
   */
  async archiveSession(rowLabel: string): Promise<void> {
    await this.openSessionMenu(rowLabel)
    await this.host
      .locator('li.session-item:not(.archived)', { hasText: rowLabel })
      .first()
      .locator('wa-dropdown-item[value="archive"]')
      .click()
  }

  /**
   * Close (delete) a session row by its displayed label, via the row's …
   * menu. Inactive sessions close immediately; active ones raise the inline
   * confirm dialog — callers that need the dialog handle it themselves.
   */
  async closeSession(rowLabel: string): Promise<void> {
    await this.openSessionMenu(rowLabel)
    await this.host
      .locator('li.session-item:not(.archived)', { hasText: rowLabel })
      .first()
      .locator('wa-dropdown-item[value="close"]')
      .click()
  }

  /**
   * The add-project dialog (wa-dialog with the folder picker + manual
   * path input). Web Awesome renders it in the top layer when open.
   */
  addDialog(): Locator {
    return this.page.locator('sebas-project-rail wa-dialog[label="Add project"]')
  }

  async openAddDialog(): Promise<void> {
    await this.addButton.click()
    // wa-dialog uses a top-layer popover: the <wa-dialog> host reads hidden,
    // but the rendered contents (heading/tree/input) are visible. Assert on
    // the visible heading instead of the host.
    await expect(
      this.addDialog().locator('h2, [role="heading"]', { hasText: 'Add project' }).first(),
    ).toBeVisible()
  }

  /**
   * The creation dialog（workbench-interaction-polish D2：项目行「+」打开，
   * agent 必选 + 两级模型 + mode）——hosted by the rail; Web Awesome renders
   * it in the top layer when open.
   */
  newSessionDialog(): Locator {
    return this.page.locator('sebas-new-session-dialog [data-testid="new-session-dialog"]')
  }

  /** The `+` trigger on a project row (hover-revealed like the … menu). */
  async openNewSessionDialog(projectName: string): Promise<void> {
    const row = this.projectRow(projectName)
    await row.hover()
    await row.locator('button[aria-label^="New session"]').click()
    await expect(
      this.newSessionDialog().locator('h2, [role="heading"]').first(),
    ).toBeVisible()
  }

  /** Pick an agent in the dialog (wa-select value + standard change event). */
  async pickDialogAgent(agent: string): Promise<void> {
    await this.newSessionDialog()
      .locator('[data-testid="dialog-agent-select"]')
      .evaluate((el, v) => {
        ;(el as unknown as { value: string }).value = v as string
        el.dispatchEvent(new Event('change', { bubbles: true }))
      }, agent)
  }

  /** Pick a permission mode ('' = agent default = omit the field). */
  async pickDialogMode(mode: string): Promise<void> {
    await this.newSessionDialog()
      .locator('[data-testid="dialog-mode-select"]')
      .evaluate((el, v) => {
        ;(el as unknown as { value: string }).value = v as string
        el.dispatchEvent(new Event('change', { bubbles: true }))
      }, mode)
  }

  /** Confirm the creation dialog. */
  async confirmNewSessionDialog(): Promise<void> {
    await this.newSessionDialog().locator('[data-testid="dialog-confirm"]').click()
  }

  /** Cancel the creation dialog (nothing is created). */
  async cancelNewSessionDialog(): Promise<void> {
    await this.newSessionDialog().locator('[data-testid="dialog-cancel"]').click()
  }

  /**
   * Add a project by typing its absolute path (the dialog's manual path
   * input — the same dialog that hosts the folder picker). The path field's
   * `@input` handler gates the Add button, so we type character-by-character
   * (`.fill` can bypass wa-input's input event in some versions).
   */
  async addProjectByPath(dir: string): Promise<void> {
    const input = this.addDialog().locator('wa-input[label="Project path"] input')
    await input.click()
    await input.pressSequentially(dir)
    await this.addDialog()
      .locator('wa-button')
      .filter({ hasText: 'Add project' })
      .click()
  }
}

/** Transcript view: bubbles, thinking folds, the unseen seam. */
export class Transcript {
  readonly page: Page
  readonly host: Locator
  readonly scroll: Locator
  readonly seam: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-transcript-view')
    this.scroll = page.locator('sebas-transcript-view .scroll')
    this.seam = page.locator('sebas-transcript-view .seam')
  }

  /** Turn blocks (assistant + user bubbles) as a countable locator. */
  turns(): Locator {
    return this.page.locator('sebas-transcript-view .turn-block')
  }

  turnWith(text: string): Locator {
    return this.page.locator('sebas-transcript-view .turn-block', { hasText: text })
  }
}

/** Review cards for gated tool calls. */
export class ReviewCards {
  readonly page: Page
  readonly region: Locator

  constructor(page: Page) {
    this.page = page
    this.region = page.locator('sebas-review-cards .review-cards[aria-label="Permission review"]')
  }

  /** Nth pending card. */
  card(index = 0): Locator {
    return this.page.locator('sebas-review-cards section.review-card').nth(index)
  }

  all(): Locator {
    return this.page.locator('sebas-review-cards section.review-card')
  }

  allowOnce(index = 0): Locator {
    return this.card(index).locator('wa-button.allow-once')
  }

  allowSession(index = 0): Locator {
    return this.card(index).locator('wa-button.allow-session')
  }

  deny(index = 0): Locator {
    return this.card(index).locator('wa-button.deny')
  }
}

/** Sessions list page (`/sessions`): create form + card grid + close dialog. */
export class SessionsPage {
  readonly page: Page
  readonly host: Locator
  readonly pageTitle: Locator
  readonly promptInput: Locator
  readonly newSessionButton: Locator
  readonly cards: Locator
  readonly closeDialog: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-sessions')
    this.pageTitle = page.locator('sebas-sessions .page-title')
    this.promptInput = page.locator('sebas-sessions wa-input[aria-label="New session prompt"] input')
    this.newSessionButton = page
      .locator('sebas-sessions form.composer wa-button')
      .filter({ hasText: 'New session' })
    this.cards = page.locator('sebas-sessions article.scard')
    this.closeDialog = page.locator('sebas-sessions wa-dialog[label="Close session"]')
  }

  /** The card whose detail link targets the given encoded key. */
  cardFor(encodedKey: string): Locator {
    return this.cards.filter({ has: this.page.locator(`a.chat[href="/sessions/${encodedKey}"]`) })
  }

  async closeCard(encodedKey: string): Promise<void> {
    await this.cardFor(encodedKey)
      .locator('wa-button[aria-label^="Close session"]')
      .click()
    // wa-dialog host is popover-hidden; assert the visible dialog copy.
    await expect(
      this.closeDialog.locator('h2, [role="heading"]', { hasText: 'Close session' }).first(),
    ).toBeVisible()
    await this.closeDialog
      .locator('wa-button')
      .filter({ hasText: 'Close session' })
      .click()
  }
}

/** Settings modal (opens from the composer's `settings →`). */
export class SettingsModal {
  readonly page: Page
  readonly host: Locator
  readonly panel: Locator
  readonly closeButton: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-settings-modal')
    this.panel = page.locator('sebas-settings-modal .panel[role="dialog"][aria-label="Settings"]')
    this.closeButton = page.locator('sebas-settings-modal button.close[aria-label="Close settings"]')
  }

  async openViaSidebar(): Promise<void> {
    // workbench-interaction-polish 4.1：composer 不再有 settings 入口——
    // Settings 归 app shell 侧栏底部（pinned footer）。
    await this.page
      .locator('sebas-app .sidebar-footer button[aria-label="Open settings"]')
      .click()
    await expect(this.panel).toBeVisible()
  }

  /**
   * Switch to a settings section by its nav label; waits for load to settle.
   * Order is the spec (settings-modal SECTIONS): Generic → Appearance →
   *〔分隔线〕Services · Users(仅 root) → Models →〔压底分隔线〕Env Vars ·
   * About. The former Settings overview and standalone Env sections are gone;
   * split-env-vars-settings-section 迁出的 Env 表住底部只读组「Env Vars」。
   */
  async openSection(
    label: 'Generic' | 'Models' | 'Services' | 'Appearance' | 'About' | 'Env Vars',
  ): Promise<void> {
    await this.panel.locator('.nav-item', { hasText: label }).click()
    await expect(
      this.panel.locator('.nav-item', { hasText: label }),
    ).toHaveAttribute('aria-current', 'true')
  }

  async close(): Promise<void> {
    await this.closeButton.click()
    await expect(this.panel).toBeHidden()
  }
}
