/**
 * Page objects for the React + AntD workbench surfaces.
 *
 * The class names are retained from the Lit suite so every legacy journey keeps
 * its import, but the selectors underneath are public React semantics: ARIA
 * roles, accessible names, and stable `data-testid` hooks. No Lit/Web Awesome
 * host tags or Shadow DOM internals are used.
 */
import { expect, type Locator, type Page } from '@playwright/test'
import { cjk, pickSelectOption } from './react'

/** Sidebar project tree: projects, per-project sessions, History group. */
export class ProjectRail {
  readonly page: Page
  readonly host: Locator
  readonly addButton: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByRole('complementary', { name: '项目导航' })
    this.addButton = this.host.getByRole('button', { name: '添加项目' })
  }

  /** A project row by displayed name (accessible name is `name + count`). */
  projectRow(name: string): Locator {
    return this.host.getByRole('button', {
      name: new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\d+$`),
    })
  }

  /** Expand a project row to reveal its sessions (the row itself toggles). */
  async expandProject(name: string): Promise<void> {
    await this.ensureProjectExpanded(name)
  }

  /** Expand a project row only when collapsed. */
  async ensureProjectExpanded(name: string): Promise<void> {
    const row = this.projectRow(name)
    await row.waitFor({ state: 'visible', timeout: 15_000 })
    for (let guard = 0; guard < 5; guard++) {
      if ((await row.getAttribute('aria-expanded')) !== 'false') return
      await row.click()
      await this.page.waitForTimeout(150)
    }
  }

  /** The History group toggle (name `历史 <count>`), with its expanded state. */
  get historyToggle(): Locator {
    return this.host.getByRole('button', { name: /^历史 \d+$/ })
  }

  /** Expand the History group (archived sessions; hidden when empty). */
  async expandHistory(): Promise<void> {
    const head = this.historyToggle
    await head.waitFor({ state: 'visible', timeout: 15_000 })
    if ((await head.getAttribute('aria-expanded')) === 'true') return
    await head.click()
    await expect(head).toHaveAttribute('aria-expanded', 'true')
  }

  /** Open a project row's overflow menu (accessible name 项目操作 <name>). */
  async openProjectMenu(name: string): Promise<void> {
    await this.host.getByRole('button', { name: `项目操作 ${name}` }).click()
    // AntD keeps previously opened dropdowns mounted while the new one
    // animates in; only the visible menu is the active surface.
    await expect(this.page.getByRole('menu').filter({ visible: true }).first()).toBeVisible()
  }

  /** Click one open overflow-menu item by its visible label. */
  async clickMenuItem(name: string): Promise<void> {
    await this.page
      .getByRole('menuitem', { name, exact: true })
      .filter({ visible: true })
      .first()
      .click()
  }

  /** Session row button by its displayed label. */
  sessionItem(label: string): Locator {
    return this.host
      .getByRole('button', { name: new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`) })
      .first()
  }

  /** The session row's overflow menu trigger (…). */
  sessionMenu(label: string): Locator {
    return this.host.getByRole('button', { name: `会话操作 ${label}` })
  }

  /** Open a session row's overflow menu and return the archive item. */
  async openSessionMenu(rowLabel: string): Promise<Locator> {
    await this.sessionMenu(rowLabel).click()
    const menu = this.page.getByRole('menuitem', { name: '归档会话' })
    await expect(menu).toBeVisible()
    return menu
  }

  archiveDialog(): Locator {
    return this.page.getByRole('dialog', { name: '归档会话' })
  }

  async openArchiveDialog(rowLabel: string): Promise<Locator> {
    await this.sessionMenu(rowLabel).click()
    await this.page.getByRole('menuitem', { name: '归档会话' }).click()
    const dialog = this.archiveDialog()
    await expect(dialog).toBeVisible()
    return dialog
  }

  async confirmArchive(): Promise<void> {
    await this.archiveDialog().getByRole('button', { name: cjk('归档') }).click()
  }

  async archiveSession(rowLabel: string): Promise<void> {
    await this.openArchiveDialog(rowLabel)
    await this.confirmArchive()
  }

  /** The add-project dialog (folder picker + manual path). */
  addDialog(): Locator {
    return this.page.getByRole('dialog', { name: '选择项目目录' })
  }

  async openAddDialog(): Promise<void> {
    await this.addButton.click()
    await expect(this.addDialog()).toBeVisible()
  }

  /** The creation dialog (rail project menu → 新建会话). */
  newSessionDialog(): Locator {
    return this.page.getByRole('dialog', { name: '新建会话' })
  }

  async openNewSessionDialog(projectName: string): Promise<void> {
    await this.host.getByRole('button', { name: `项目操作 ${projectName}` }).click()
    await this.page.getByRole('menuitem', { name: '新建会话' }).click()
    await expect(this.newSessionDialog()).toBeVisible()
  }

  async pickDialogAgent(agent: string): Promise<void> {
    await pickSelectOption(
      this.page,
      this.newSessionDialog().getByRole('combobox', { name: 'Agent' }),
      agent,
    )
  }

  /** Provider level of the combined model selector (selects its first pair). */
  async pickDialogProvider(provider: string): Promise<void> {
    await pickSelectOption(
      this.page,
      this.newSessionDialog().getByRole('combobox', { name: '模型' }),
      provider,
    )
  }

  async pickDialogModel(model: string): Promise<void> {
    await pickSelectOption(
      this.page,
      this.newSessionDialog().getByRole('combobox', { name: '模型' }),
      model,
    )
  }

  async pickDialogMode(mode: string): Promise<void> {
    await pickSelectOption(
      this.page,
      this.newSessionDialog().getByRole('combobox', { name: '权限模式' }),
      mode,
    )
  }

  async confirmNewSessionDialog(): Promise<void> {
    const dialog = this.newSessionDialog()
    // React does not preselect an agent (the Lit dialog did). Journeys that
    // only care about "create the placeholder" keep their old intent by
    // filling the default agent; journeys that exercise agent choice pick it
    // explicitly first. AntD 6 renders the selected label as text inside
    // `.ant-select-content` (the inner search input stays empty), so probe
    // that instead of inputValue().
    const agent = dialog.getByRole('combobox', { name: 'Agent' })
    const selected = await agent.evaluate((el) => {
      const root = el.closest('.ant-select')
      const content = root?.querySelector('.ant-select-content')
      // AntD 6 keeps the placeholder INSIDE `.ant-select-content`; treat an
      // unselected select as empty instead of reading the placeholder text.
      if (!content || content.querySelector('.ant-select-placeholder')) return ''
      return (content.textContent ?? '').trim()
    })
    if (selected === '') {
      await this.pickDialogAgent('claude')
    }
    const create = dialog.getByRole('button', { name: cjk('创建') })
    // Picking an agent kicks off the slot/catalog load; AntD's loading button
    // swallows clicks for that window (same failure mode as the add-folder
    // submit). Settle before submitting.
    await expect(create).not.toHaveClass(/ant-btn-loading/)
    await expect(create).toBeEnabled()
    await create.click()
  }

  async cancelNewSessionDialog(): Promise<void> {
    await this.newSessionDialog().getByRole('button', { name: cjk('取消') }).click()
  }

  /** Add a project by typing its absolute path and using it. */
  async addProjectByPath(dir: string): Promise<void> {
    const dialog = this.addDialog()
    await dialog.getByRole('textbox', { name: '项目路径' }).fill(dir)
    const use = dialog.getByRole('button', { name: '使用此路径' })
    // The path field runs a debounced as-you-type precheck and the submit
    // carries AntD's loading state for its whole window; a loading AntD
    // button swallows clicks without reporting anything. Wait for the check
    // to settle (and the field's state to commit) before submitting.
    await expect(use).not.toHaveClass(/ant-btn-loading/)
    await expect(use).toBeEnabled()
    await use.click()
  }
}

/** Transcript view: units (bubbles), folds, the unseen seam. */
export class Transcript {
  readonly page: Page
  readonly host: Locator
  readonly scroll: Locator
  readonly seam: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByTestId('react-transcript')
    this.scroll = page.getByTestId('conversation-pane')
    this.seam = this.host.getByRole('separator', { name: /未读分界/ })
  }

  turns(): Locator {
    return this.host.locator('[data-unit-key]')
  }

  turnWith(text: string): Locator {
    return this.turns().filter({ hasText: text })
  }
}

/** Approval cards for gated tool calls. */
export class ReviewCards {
  readonly page: Page
  readonly region: Locator

  constructor(page: Page) {
    this.page = page
    this.region = page.getByLabel('待审批工具调用')
  }

  card(index = 0): Locator {
    return this.region.getByRole('alertdialog').nth(index)
  }

  all(): Locator {
    return this.region.getByRole('alertdialog')
  }

  allowOnce(index = 0): Locator {
    return this.card(index).getByRole('button', { name: /允\s*许\s*一\s*次/ })
  }

  allowSession(index = 0): Locator {
    return this.card(index).getByRole('button', { name: /本\s*会\s*话\s*允\s*许/ })
  }

  deny(index = 0): Locator {
    return this.card(index).getByRole('button', { name: /拒\s*绝/ })
  }
}

/** Sessions list page (`/sessions`). */
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
    this.host = page.getByTestId('sessions-page')
    this.pageTitle = this.host.getByRole('heading', { level: 3, name: '会话总览' })
    this.promptInput = this.page.getByRole('dialog', { name: '新建会话' }).getByRole('textbox', {
      name: '任务描述',
    })
    this.newSessionButton = this.host.getByRole('button', { name: cjk('新建会话') })
    this.cards = this.host.locator('.ant-card')
    this.closeDialog = this.page.getByRole('dialog', { name: '归档会话' })
  }

  /**
   * The card for a session. The React card does not expose the encoded key, so
   * callers that only know the key must pass the display label instead.
   */
  cardFor(label: string): Locator {
    return this.cards.filter({ hasText: label })
  }

  async closeCard(label: string): Promise<void> {
    await this.cardFor(label).getByRole('button', { name: cjk('归档') }).click()
    await expect(this.closeDialog).toBeVisible()
    await this.closeDialog.getByRole('button', { name: cjk('归档') }).click()
  }
}

/** Settings window (floating dialog, opens from the rail footer). */
export class SettingsModal {
  readonly page: Page
  readonly host: Locator
  readonly panel: Locator
  readonly closeButton: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByRole('region', { name: '设置' })
    this.panel = this.host
    // The close control belongs to the window chrome (Modal header), which is
    // outside the labelled region.
    this.closeButton = page.getByRole('dialog', { name: '设置' }).getByRole('button', { name: '关闭设置' })
  }

  async openViaSidebar(): Promise<void> {
    await this.page.getByRole('complementary', { name: '项目导航' }).getByRole('button', { name: /设\s*置/ }).click()
    await expect(this.panel).toBeVisible()
  }

  async openSection(
    label:
      | '通用'
      | '模型'
      | '别名'
      | 'Agent'
      | '技能'
      | '服务'
      | '用户'
      | '外观'
      | '关于'
      | '环境变量',
  ): Promise<void> {
    const button = this.panel.getByRole('button', { name: label, exact: true })
    await button.click()
    await expect(button).toHaveAttribute('aria-current', 'page')
  }

  async close(): Promise<void> {
    await this.closeButton.click()
    await expect(this.panel).toBeHidden()
  }
}
