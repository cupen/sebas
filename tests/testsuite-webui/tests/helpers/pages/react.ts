/**
 * Page objects for the React + AntD workbench (redesign-webui-with-react-antd
 * 5.1). Selectors are the product's public semantics — ARIA roles, accessible
 * names and the app's stable `data-testid` hooks — never Lit/Web Awesome
 * internals or Shadow DOM. Two AntD interaction seams are documented inline:
 * Select options expose an off-screen role=option mirror, and two-character
 * Chinese buttons get an inter-character space in their accessible name.
 */
import { expect, type Locator, type Page } from '@playwright/test'

/** Escape a literal for use inside a RegExp. */
export function escapeRe(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * AntD 的两个中文字符按钮会在可访问名里插入空格（「发 送」「归 档」）——
 * 文案断言一律用宽松正则，避免把组件库的排版细节固化进测试。
 */
export function cjk(label: string): RegExp {
  return new RegExp(`^${[...label].map(escapeRe).join('\\s*')}$`)
}

/** Pick an option from an AntD Select opened by `combobox`. */
export async function pickSelectOption(
  page: Page,
  combobox: Locator,
  option: string,
): Promise<void> {
  await combobox.click()
  // AntD keeps a 0×0 role=listbox mirror for a11y (not clickable) while the
  // visible option rows are `.ant-select-item-option` — the current component
  // library's own surface, not a previous framework's internals.
  const dropdown = page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
  const options = dropdown.locator('.ant-select-item-option')
  // Model options read `provider / model` and mode options add a Chinese
  // gloss, so an exact prefix is not always available. Prefer the exact
  // option, then fall back to a substring match.
  const exact = options.filter({ hasText: new RegExp(`^${escapeRe(option)}$`) })
  const target = (await exact.count()) > 0 ? exact : options.filter({ hasText: option })
  await target.first().click()
}

/** The app shell: nav rail/drawer, theme, user menu, settings entry. */
export class ReactShell {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get app(): Locator {
    return this.page.getByTestId('react-shell')
  }

  get rail(): Locator {
    return this.page.getByRole('complementary', { name: '项目导航' })
  }

  get nav(): Locator {
    return this.page.getByRole('navigation', { name: '主导航' })
  }

  navButton(name: string): Locator {
    return this.nav.getByRole('button', { name })
  }

  /** Top-bar connection indicator (role=status, text 已连接 / 连接已断开). */
  get connection(): Locator {
    return this.page.getByRole('banner').getByRole('status')
  }

  get settingsButton(): Locator {
    return this.rail.getByRole('button', { name: /设\s*置/ })
  }

  /** The floating settings window (AntD Modal titled 设置). */
  get settingsDialog(): Locator {
    return this.page.getByRole('dialog', { name: '设置' })
  }

  get userMenu(): Locator {
    return this.rail.getByRole('button', { name: '用户菜单' })
  }

  get narrowMenuToggle(): Locator {
    return this.page.getByRole('button', { name: /菜\s*单/ })
  }

  /** The narrow-viewport navigation drawer (AntD Drawer dialog "导航"). */
  get drawer(): Locator {
    return this.page.getByRole('dialog', { name: '导航' })
  }

  async openDrawer(): Promise<void> {
    await this.narrowMenuToggle.click()
    await expect(this.drawer).toBeVisible()
  }

  async openSettings(): Promise<void> {
    await this.settingsButton.click()
    await expect(this.settings).toBeVisible()
  }

  get settings(): Locator {
    return this.page.getByRole('region', { name: '设置' })
  }

  async logout(): Promise<void> {
    await this.userMenu.click()
    await this.page.getByRole('menuitem', { name: '退出登录' }).click()
  }

  /** Root theme mode resolved by the provider (html[data-theme]). */
  async resolvedTheme(): Promise<string | null> {
    return this.page.locator('html').getAttribute('data-theme')
  }

  async setTheme(label: '跟随系统' | '浅色' | '深色'): Promise<void> {
    // Appearance lives in the settings window now (the rail switch is gone):
    // drive the real journey instead of poking localStorage.
    const settings = new ReactSettings(this.page)
    await this.openSettings()
    await settings.openSection('外观')
    // Two-character CJK buttons carry an inter-character space in their
    // accessible name (AntD typography): match through the shared helper.
    await this.settings.getByRole('button', { name: cjk(label) }).click()
    const expected = label === '跟随系统' ? null : label === '浅色' ? 'light' : 'dark'
    if (expected !== null) {
      await expect(this.page.locator('html')).toHaveAttribute('data-theme', expected)
    }
    await settings.close()
  }
}

/** Project rail: project rows, session rows, history, add-project. */
export class ReactRail {
  readonly page: Page
  /** Override for the narrow-viewport drawer, where the rail has no aside. */
  private readonly root: Locator | null

  constructor(page: Page, root?: Locator) {
    this.page = page
    this.root = root ?? null
  }

  get host(): Locator {
    return this.root ?? this.page.getByRole('complementary', { name: '项目导航' })
  }

  get addProjectButton(): Locator {
    return this.host.getByRole('button', { name: '添加项目' })
  }

  /** Project row by display name (name + session count). */
  projectRow(name: string): Locator {
    return this.host.getByRole('button', { name: new RegExp(`^${escapeRe(name)}\\s*\\d+$`) })
  }

  /** The project's overflow menu trigger (accessible name 项目操作 <name>). */
  projectMenu(name: string): Locator {
    return this.host.getByRole('button', { name: `项目操作 ${name}` })
  }

  /** A session row by its displayed label (row button starts with the label). */
  sessionRow(label: string): Locator {
    return this.host.getByRole('button', { name: new RegExp(`^${escapeRe(label)}`) }).first()
  }

  /**
   * Expand a project row only when collapsed. The row is both the select and
   * the toggle, so a blind click on an already-expanded project would hide
   * the sessions the journey is about to click.
   */
  async ensureProjectExpanded(name: string): Promise<void> {
    const row = this.projectRow(name)
    await row.waitFor({ state: 'visible', timeout: 15_000 })
    for (let guard = 0; guard < 5; guard++) {
      if ((await row.getAttribute('aria-expanded')) !== 'false') return
      await row.click()
      await this.page.waitForTimeout(150)
    }
  }

  sessionMenu(label: string): Locator {
    return this.host.getByRole('button', { name: `会话操作 ${label}` })
  }

  /** The History group toggle (name 历史 <count>), with its expanded state. */
  get historyToggle(): Locator {
    return this.host.getByRole('button', { name: /^历史 \d+$/ })
  }

  async expandHistory(): Promise<void> {
    if ((await this.historyToggle.getAttribute('aria-expanded')) === 'true') return
    await this.historyToggle.click()
    await expect(this.historyToggle).toHaveAttribute('aria-expanded', 'true')
  }

  /** An archived row (button whose name carries the archive timestamp). */
  archivedRow(label: string): Locator {
    return this.host.getByRole('button', { name: new RegExp(`^${escapeRe(label)}\\s*归档于`) })
  }

  restoreButton(label: string): Locator {
    return this.host.getByRole('button', { name: `恢复会话 ${label}` })
  }

  /** Drive a project/session menu item and wait for it to be actionable. */
  async clickMenuItem(name: string): Promise<void> {
    await this.page.getByRole('menuitem', { name, exact: true }).click()
  }

  async openProjectMenu(name: string): Promise<void> {
    await this.projectMenu(name).click()
    await expect(this.page.getByRole('menu')).toBeVisible()
  }

  async openSessionMenu(label: string): Promise<void> {
    await this.sessionMenu(label).click()
    await expect(this.page.getByRole('menu')).toBeVisible()
  }

  /** Open the archive confirm dialog for a session row. */
  async openArchiveDialog(label: string): Promise<Locator> {
    await this.openSessionMenu(label)
    await this.clickMenuItem('归档会话')
    const dialog = this.page.getByRole('dialog', { name: '归档会话' })
    await expect(dialog).toBeVisible()
    return dialog
  }
}

/** The workbench main area: landing page or focused conversation. */
export class ReactWorkbench {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get home(): Locator {
    return this.page.getByTestId('workbench-home')
  }

  get sessionPage(): Locator {
    return this.page.getByTestId('workbench-session')
  }

  /** Focused-session title (h3) — set from the label/preview truth. */
  get title(): Locator {
    // The conversation pane renders its loading skeleton as an h3 too; the
    // session header is the first one under the page root.
    return this.sessionPage.locator('header').first().getByRole('heading', { level: 3 })
  }

  /** The raw encoded session key rendered as <code> under the title. */
  get encodedKey(): Locator {
    return this.sessionPage.locator('code').first()
  }

  get conversation(): Locator {
    return this.page.getByTestId('conversation-pane')
  }

  get transcript(): Locator {
    return this.page.getByTestId('react-transcript')
  }

  /** All rendered units (operator/agent/error/notice/system) in order. */
  get units(): Locator {
    return this.transcript.locator('[data-unit-key]')
  }

  unitWith(text: string): Locator {
    return this.units.filter({ hasText: text })
  }

  /** The operator bubble carrying `text` (role label 你). */
  operatorUnit(text: string): Locator {
    return this.unitWith(text).filter({ has: this.page.getByText('你', { exact: true }) })
  }

  get composer(): Locator {
    return this.page.getByTestId('composer')
  }

  get composerTextarea(): Locator {
    return this.composer.getByRole('textbox', { name: '输入消息' })
  }

  get modeSelect(): Locator {
    return this.composer.getByRole('combobox', { name: '权限模式' })
  }

  get modelSelect(): Locator {
    return this.composer.getByRole('combobox', { name: '会话模型' })
  }

  /** Submit/stop control; `data-state` is the product's state machine hook. */
  get submitControl(): Locator {
    return this.composer.locator('button[data-state]')
  }

  get pendingQueue(): Locator {
    return this.composer.getByLabel('待执行提交')
  }

  get jumpToLatest(): Locator {
    return this.page.getByRole('button', { name: '跳到最新' })
  }

  get seam(): Locator {
    return this.transcript.getByRole('separator', { name: /未读分界/ })
  }

  get approvals(): Locator {
    return this.page.getByLabel('待审批工具调用')
  }

  approvalCards(): Locator {
    return this.approvals.getByRole('alertdialog')
  }

  approvalCard(tool: string): Locator {
    return this.page.getByRole('alertdialog', { name: `待审批 ${tool}` })
  }

  async send(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }

  /** Open every collapsed process fold (details/summary) in the transcript. */
  async expandAllFolds(): Promise<void> {
    const collapsed = this.transcript.locator('details:not([open]) > summary')
    for (let guard = 0; guard < 20; guard++) {
      if ((await collapsed.count()) === 0) return
      await collapsed.first().click()
    }
  }

  /** Fold body text (tool result etc.) is present after expanding the folds. */
  async expectFoldedText(text: string, timeout = 20_000): Promise<void> {
    await expect
      .poll(
        async () => {
          await this.expandAllFolds()
          return this.unitWith(text).count()
        },
        { timeout },
      )
      .toBeGreaterThan(0)
  }

  /** Focused-session status chip text (e.g. 完成/等待你处理). */
  async statusText(): Promise<string> {
    return (await this.sessionPage.innerText()).includes('等待你处理')
      ? '等待你处理'
      : (await this.sessionPage.innerText()).match(/启动中|排队中|运行中|完成|失败|休眠/)?.[0] ?? ''
  }
}

/** Session creation dialog (rail project menu → 新建会话). */
export class ReactSessionCreateDialog {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get dialog(): Locator {
    return this.page.getByRole('dialog', { name: '新建会话' })
  }

  get agentSelect(): Locator {
    return this.dialog.getByRole('combobox', { name: 'Agent' })
  }

  get projectSelect(): Locator {
    return this.dialog.getByRole('combobox', { name: '项目' })
  }

  get modeSelect(): Locator {
    return this.dialog.getByRole('combobox', { name: '权限模式' })
  }

  get promptInput(): Locator {
    return this.dialog.getByRole('textbox', { name: '任务描述' })
  }

  async pickAgent(agent: string): Promise<void> {
    await pickSelectOption(this.page, this.agentSelect, agent)
  }

  async create(): Promise<void> {
    await this.dialog.getByRole('button', { name: /创\s*建/ }).click()
  }

  async cancel(): Promise<void> {
    await this.dialog.getByRole('button', { name: /取\s*消/ }).click()
  }
}

/** Settings workspace (no dedicated URL). */
export class ReactSettings {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get region(): Locator {
    return this.page.getByRole('region', { name: '设置' })
  }

  get dialog(): Locator {
    return this.page.getByRole('dialog', { name: '设置' })
  }

  get nav(): Locator {
    return this.region.getByRole('complementary', { name: '设置分区' })
  }

  section(name: string): Locator {
    return this.nav.getByRole('button', { name })
  }

  /** The window's own close control (in the Modal header, outside the region). */
  get closeButton(): Locator {
    return this.dialog.getByRole('button', { name: '关闭设置' })
  }

  async openSection(name: string): Promise<void> {
    await this.section(name).click()
    await expect(this.section(name)).toHaveAttribute('aria-current', 'page')
  }

  async close(): Promise<void> {
    await this.closeButton.click()
    await expect(this.region).toBeHidden()
  }

  /** Settings → Aliases inline draft fields (kept across section switches). */
  get aliasInput(): Locator {
    return this.region.getByLabel('别名', { exact: true })
  }

  get aliasUpstreamInput(): Locator {
    return this.region.getByLabel('上游模型')
  }
}

/** `/sessions` overview page. */
export class ReactSessionsPage {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get pageRoot(): Locator {
    return this.page.getByTestId('sessions-page')
  }

  get title(): Locator {
    return this.pageRoot.getByRole('heading', { level: 3, name: '会话总览' })
  }

  card(label: string): Locator {
    return this.pageRoot.locator('.ant-card').filter({ hasText: label })
  }

  async focus(label: string): Promise<void> {
    await this.card(label).getByRole('button', { name: /聚\s*焦/ }).click()
  }
}

/** `/usage` page. */
export class ReactUsagePage {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get pageRoot(): Locator {
    return this.page.getByTestId('usage-page')
  }

  get sourceGroup(): Locator {
    return this.page.getByRole('radiogroup', { name: '数据来源' })
  }

  get granularityGroup(): Locator {
    return this.page.getByRole('radiogroup', { name: '粒度' })
  }

  get dimensionGroup(): Locator {
    return this.page.getByRole('radiogroup', { name: '维度' })
  }
}

/** Login gate (auth-on form). */
export class ReactLogin {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get heading(): Locator {
    return this.page.getByRole('heading', { name: '登录以继续' })
  }

  get username(): Locator {
    return this.page.getByRole('textbox', { name: '用户名' })
  }

  get password(): Locator {
    return this.page.getByRole('textbox', { name: /^密码/ })
  }

  get submit(): Locator {
    return this.page.getByRole('button', { name: /登\s*录/ })
  }

  get error(): Locator {
    return this.page.getByRole('alert')
  }

  async signIn(username: string, password: string): Promise<void> {
    await this.username.fill(username)
    await this.password.fill(password)
    await this.submit.click()
  }
}

/** First-run root setup gate (auth on, zero users). */
export class ReactSetup {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get heading(): Locator {
    return this.page.getByRole('heading', { name: '创建管理员账户' })
  }

  get username(): Locator {
    return this.page.getByRole('textbox', { name: '用户名' })
  }

  get password(): Locator {
    return this.page.getByLabel(/^密码（至少/)
  }

  get confirm(): Locator {
    return this.page.getByLabel('确认密码')
  }

  get submit(): Locator {
    return this.page.getByRole('button', { name: '创建并进入' })
  }

  get error(): Locator {
    return this.page.getByRole('alert')
  }

  async createRoot(username: string, password: string): Promise<void> {
    await this.username.fill(username)
    await this.password.fill(password)
    await this.confirm.fill(password)
    await this.submit.click()
  }
}

/** Project directory picker (rail → 添加项目). */
export class ReactFolderPicker {
  readonly page: Page

  constructor(page: Page) {
    this.page = page
  }

  get dialog(): Locator {
    return this.page.getByRole('dialog', { name: '选择项目目录' })
  }

  get list(): Locator {
    return this.dialog.getByLabel('子目录')
  }

  get newFolderName(): Locator {
    return this.dialog.getByLabel('新建文件夹名称')
  }

  async chooseCurrentDirectory(): Promise<void> {
    await this.dialog.getByRole('button', { name: '选择此目录' }).click()
  }
}
