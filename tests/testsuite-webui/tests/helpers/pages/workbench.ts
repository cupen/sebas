/**
 * Page objects for the React workbench shell and focused conversation.
 *
 * Public class/property names are retained from the Lit suite; every locator is
 * React/AntD public semantics (roles, accessible names, stable testids).
 */
import { expect, type Locator, type Page } from '@playwright/test'

const STATUS_TEXT: Record<string, string> = {
  starting: '启动中',
  queued: '排队中',
  working: '运行中',
  waiting: '等待你处理',
  done: '完成',
  failed: '失败',
  dormant: '休眠',
}

/** The app shell. */
export class AppShell {
  readonly page: Page
  readonly app: Locator
  readonly brand: Locator
  readonly outlet: Locator

  constructor(page: Page) {
    this.page = page
    this.app = page.getByTestId('react-shell')
    this.brand = this.app.getByRole('button', { name: /sebas/ }).first()
    this.outlet = this.app.locator('main')
  }

  /** Main navigation (rail labels: 工作台 / 会话总览 / 用量 / 设置). */
  get nav(): Locator {
    return this.app.getByRole('navigation', { name: '主导航' })
  }

  async logout(username: string): Promise<void> {
    await this.app.getByRole('button', { name: '用户菜单' }).click()
    await expect(this.page.getByRole('menuitem', { name: /退出登录/ })).toContainText(username)
    await this.page.getByRole('menuitem', { name: /退出登录/ }).click()
  }
}

/** The workbench main area (home or focused session). */
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
    this.host = page.locator(
      ':is([data-testid="workbench-session"], [data-testid="workbench-home"])',
    )
    this.projectHeader = page.getByRole('banner')
    this.noProjectSelected = this.host.getByText('从左侧选择项目与会话。')
    this.emptyStream = page.getByTestId('workbench-home').getByText('还没有聚焦会话')
    this.composer = page.getByTestId('composer')
    this.composerTextarea = this.composer.getByRole('textbox', { name: '输入消息' })
    this.sendButton = this.composer.locator('button[data-state]')
    this.submitControl = this.composer.locator('button[data-state]')
    this.modelChip = this.composer.getByRole('combobox', { name: '会话模型' })
    this.modelMenu = this.page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)')
    this.noFocusHint = this.emptyStream
    this.reachabilityWarning = this.composer.getByText(/核心不可达/)
  }

  async sendPrompt(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }

  submitState(): Promise<string | null> {
    return this.submitControl.getAttribute('data-state')
  }
}

/** The focused session on the workbench. */
export class FocusedSession {
  readonly page: Page
  readonly host: Locator
  readonly sessionHead: Locator
  readonly statusBadge: Locator
  readonly chatId: Locator
  readonly transcript: Locator
  readonly emptyConversation: Locator
  readonly unavailableNote: Locator
  readonly composerTextarea: Locator
  readonly sendButton: Locator
  readonly modelPick: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByTestId('workbench-session')
    this.sessionHead = this.host.locator('header').first()
    this.statusBadge = this.sessionHead.locator('.ant-tag').first()
    this.chatId = this.sessionHead.locator('code').first()
    this.transcript = page.getByTestId('react-transcript')
    this.emptyConversation = this.transcript.getByText('会话还没有消息')
    this.unavailableNote = page.getByRole('alert').filter({ hasText: '会话加载失败' })
    this.composerTextarea = page
      .getByTestId('composer')
      .getByRole('textbox', { name: '输入消息' })
    this.sendButton = page.getByTestId('composer').locator('button[data-state]')
    this.modelPick = page.getByTestId('composer')
  }

  async statusSlug(): Promise<string> {
    const text = (await this.statusBadge.innerText()).trim()
    return Object.entries(STATUS_TEXT).find(([, label]) => label === text)?.[0] ?? text
  }

  async expectStatus(slug: string, timeout = 15_000): Promise<void> {
    await expect(this.statusBadge).toHaveText(STATUS_TEXT[slug] ?? slug, { timeout })
  }

  /** Expand rail projects (kept as a no-op compatibility hook). */
  async expandAllProjects(): Promise<void> {
    return
  }

  async expectSessionStatus(label: string, slug: string, timeout = 15_000): Promise<void> {
    const row = this.page
      .getByRole('complementary', { name: '项目导航' })
      .getByRole('button', {
        name: new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`),
      })
      .first()
    await expect(row).toContainText(STATUS_TEXT[slug] ?? slug, { timeout })
  }

  bubbles(): Locator {
    return this.transcript.locator('[data-unit-key]')
  }

  turnWith(text: string): Locator {
    return this.bubbles().filter({ hasText: text })
  }

  userTurn(text: string): Locator {
    return this.turnWith(text).filter({ has: this.page.getByText('你', { exact: true }) })
  }

  agentTurn(text: string): Locator {
    return this.turnWith(text).filter({ has: this.page.getByText('agent', { exact: true }) })
  }

  /** Collapsed process/tool folds are React `<details>` elements. */
  processFold(): Locator {
    return this.transcript.locator('details')
  }

  processItems(): Locator {
    return this.transcript.locator('details')
  }

  async expandAllFolds(): Promise<void> {
    const collapsed = this.transcript.locator('details:not([open]) > summary')
    for (let guard = 0; guard < 20; guard++) {
      if ((await collapsed.count()) === 0) return
      await collapsed.first().click()
    }
  }

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

  async sendFollowUp(text: string): Promise<void> {
    await this.composerTextarea.fill(text)
    await this.composerTextarea.press('Enter')
  }
}
