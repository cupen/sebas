/**
 * React 入口工作台旅程（redesign-webui-with-react-antd 5.1/5.2）。
 *
 * 与主套件同一套一次性沙箱与 fake-claude 桩，页面由 **React 生产入口**
 * 驱动（`playwright.react.config.ts`：Vite dev server 提供 `/` 并代理到
 * `invoke testsuite-webui-server`）。选择器只用角色、可访问名称与稳定
 * `data-testid`。
 *
 * 覆盖：首屏骨架、项目上下文建房、深链切换不串话、归档/历史/恢复、
 * 审批单次允许、待执行提交的归档确认、Markdown 长内容、设置工作区不产生
 * 退休路由且不丢输入、会话总览/用量页。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  cjk,
  ensureSceneProject,
  ErrorCollector,
  pickSelectOption,
  ReactRail,
  ReactSessionCreateDialog,
  ReactSessionsPage,
  ReactSettings,
  ReactShell,
  ReactUsagePage,
  ReactWorkbench,
  resetState,
  waitStatus,
} from './helpers/index'
import type { APIRequestContext } from '@playwright/test'

test.describe('React 工作台旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  /** Seed a converged session through the public API (server truth). */
  async function idleSession(
    request: APIRequestContext,
    prompt: string,
  ): Promise<{ key: string; project: { id: string; name: string } }> {
    const project = await ensureSceneProject(request)
    const key = await createSession(request, { prompt, projectId: project.id })
    await waitStatus(request, key, ['done'])
    return { key, project }
  }

  test.describe('首屏与信息架构', () => {
    test('renders the shell, rail and honest empty states (desktop)', async ({ page }) => {
      await resetState(page.request)
      const shell = new ReactShell(page)
      const workbench = new ReactWorkbench(page)

      await page.goto('/')

      await expect(shell.app).toBeVisible()
      await expect(shell.rail).toBeVisible()
      await expect(shell.nav.getByRole('button')).toHaveCount(3)
      await expect(shell.navButton('工作台')).toHaveAttribute('aria-current', 'page')
      // Live WS reached the sandbox core (real HTTP/WS, not a mock).
      await expect(shell.connection).toHaveText('已连接', { timeout: 15_000 })
      await expect(workbench.home).toBeVisible()
      await expect(workbench.home.getByRole('heading', { level: 3 })).toHaveText('工作台')
      // Fresh sandbox: no projects, no sessions, honest hint.
      await expect(shell.rail.getByText('还没有项目')).toBeVisible()
      await expect(shell.settingsButton).toBeVisible()

      // Unknown paths fall back to the workbench (existing SPA semantics).
      await page.goto('/no-such-page')
      await expect(shell.app).toBeVisible()
      await expect(workbench.home).toBeVisible()

      const overflow = await page.evaluate(
        () => document.scrollingElement!.scrollWidth - document.scrollingElement!.clientWidth,
      )
      expect(overflow).toBeLessThanOrEqual(1)
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('会话创建与深链', () => {
    test('creates a session from the project context and keeps it across reload', async ({
      page,
    }) => {
      await resetState(page.request)
      const project = await ensureSceneProject(page.request)
      const shell = new ReactShell(page)
      const rail = new ReactRail(page)
      const dialog = new ReactSessionCreateDialog(page)
      const workbench = new ReactWorkbench(page)

      await page.goto('/')
      await rail.openProjectMenu(project.name)
      await rail.clickMenuItem('新建会话')
      await expect(dialog.dialog).toBeVisible()
      // Agent is an explicit choice, never implicit.
      await pickSelectOption(page, dialog.agentSelect, 'claude')
      await dialog.promptInput.fill('react create journey')
      await dialog.create()

      await expect(page).toHaveURL(/\/sessions\/[^/]+$/)
      await expect(workbench.sessionPage).toBeVisible({ timeout: 15_000 })
      await expect(workbench.title).toHaveText('react create journey', { timeout: 15_000 })
      // Operator bubble + agent reply are real WS projections.
      await expect(workbench.operatorUnit('react create journey')).toBeVisible()
      await expect(workbench.transcript).toContainText('hello', { timeout: 20_000 })
      await expect.poll(async () => (await workbench.statusText()), { timeout: 20_000 }).toBe('完成')

      // Reload restores the same deep link and transcript.
      const url = page.url()
      await page.reload()
      await expect(page).toHaveURL(url)
      await expect(workbench.title).toHaveText('react create journey', { timeout: 15_000 })
      await expect(workbench.operatorUnit('react create journey')).toBeVisible()
      await expect(workbench.transcript).toContainText('hello', { timeout: 20_000 })

      expect(collector.clean()).toEqual([])
    })

    test('deep-link switching never leaks model, transcript or draft across sessions', async ({
      page,
    }) => {
      await resetState(page.request)
      const alpha = await idleSession(page.request, 'alpha deeplink')
      const bravo = await idleSession(page.request, 'bravo deeplink')
      const rail = new ReactRail(page)
      const workbench = new ReactWorkbench(page)

      await page.goto(`/sessions/${alpha.key}`)
      await expect(workbench.title).toHaveText('alpha deeplink', { timeout: 15_000 })
      await expect(workbench.encodedKey).toHaveText(alpha.key)
      await expect(workbench.transcript).toContainText('hello')
      await workbench.composerTextarea.fill('draft-alpha')

      // Switch inside the same workbench (rail row), not another detail page.
      await rail.sessionRow('bravo deeplink').click()
      await expect(page).toHaveURL(new RegExp(`/sessions/${bravo.key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`))
      await expect(workbench.title).toHaveText('bravo deeplink', { timeout: 15_000 })
      await expect(workbench.transcript).toContainText('hello')
      await expect(workbench.transcript).not.toContainText('alpha deeplink')
      // Unsent input belongs to the session, not to the panel instance.
      await expect(workbench.composerTextarea).toHaveValue('')

      // Direct deep link back to the first session.
      await page.goto(`/sessions/${alpha.key}`)
      await expect(workbench.title).toHaveText('alpha deeplink', { timeout: 15_000 })
      await expect(workbench.transcript).toContainText('alpha deeplink')
      await expect(workbench.transcript).not.toContainText('bravo deeplink')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('归档与历史', () => {
    test('archives from the row menu, shows the read-only snapshot and restores', async ({
      page,
    }) => {
      await resetState(page.request)
      // The archive list is append-only across runs; a unique label keeps the
      // journey independent of earlier archived sessions.
      const label = `idle archive ${Date.now()}`
      await idleSession(page.request, label)
      const rail = new ReactRail(page)

      await page.goto('/')
      const dialog = await rail.openArchiveDialog(label)
      await expect(dialog.getByText(/归档会终止子进程/)).toBeVisible()
      await dialog.getByRole('button', { name: cjk('归档') }).click()

      // Moved out of the active list into history.
      await expect(rail.historyToggle).toBeVisible({ timeout: 15_000 })
      await rail.expandHistory()
      await expect(rail.archivedRow(label)).toBeVisible()

      // Archived detail is a read-only snapshot: transcript text, no composer.
      await rail.archivedRow(label).click()
      const detail = page.getByRole('dialog', { name: `归档会话：${label}` })
      await expect(detail).toBeVisible()
      await expect(detail).toContainText('hello')
      await expect(detail.getByTestId('composer')).toHaveCount(0)
      await page.keyboard.press('Escape')
      await expect(detail).toBeHidden()

      // Restore brings the session back with its transcript.
      await rail.restoreButton(label).click()
      await expect(rail.archivedRow(label)).toHaveCount(0, { timeout: 15_000 })
      await expect(rail.sessionRow(label)).toBeVisible({ timeout: 15_000 })

      expect(collector.clean()).toEqual([])
    })

    test('archive confirmation names the pending submissions it will discard', async ({
      page,
    }) => {
      await resetState(page.request)
      const { key } = await idleSession(page.request, 'idle pending')
      const workbench = new ReactWorkbench(page)
      const rail = new ReactRail(page)

      // Hold the turn at an approval gate, then queue one follow-up.
      await page.goto(`/sessions/${key}`)
      await workbench.send('perm')
      await expect(workbench.approvalCard('Bash')).toBeVisible({ timeout: 20_000 })
      const queued = await page.request.post(`/api/sessions/${key}/message`, {
        data: { message: 'queued follow-up' },
      })
      expect(queued.ok()).toBe(true)

      await page.goto('/')
      const dialog = await rail.openArchiveDialog('idle pending')
      await expect(dialog.getByText(/1 条待执行提交/)).toBeVisible({ timeout: 15_000 })
      await dialog.getByRole('button', { name: cjk('取消') }).click()
      await expect(dialog).toBeHidden()

      // Session is still alive (no accidental archive).
      await expect(rail.sessionRow('idle pending')).toBeVisible()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('审批与待办', () => {
    test('allow-once resolves only the current request and the turn completes', async ({
      page,
    }) => {
      await resetState(page.request)
      const { key } = await idleSession(page.request, 'idle approval')
      const workbench = new ReactWorkbench(page)

      await page.goto(`/sessions/${key}`)
      await workbench.send('perm')
      const card = workbench.approvalCard('Bash')
      await expect(card).toBeVisible({ timeout: 20_000 })
      await expect(card).toContainText('rm -rf /')
      await expect(workbench.approvalCards()).toHaveCount(1)

      await card.getByRole('button', { name: cjk('允许一次') }).click()
      await expect(workbench.approvalCards()).toHaveCount(0, { timeout: 15_000 })
      await expect.poll(async () => (await workbench.statusText()), { timeout: 20_000 }).toBe('完成')

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('长转录与设置工作区', () => {
    test('rich markdown stays readable and never overflows the page', async ({ page }) => {
      await resetState(page.request)
      const { key } = await idleSession(page.request, 'idle markdown')
      const workbench = new ReactWorkbench(page)

      await page.goto(`/sessions/${key}`)
      await workbench.send('table')
      const table = workbench.transcript.locator('table').first()
      await expect(table).toBeVisible({ timeout: 20_000 })
      // The deterministic rich reply is a 160-column GFM table + long code.
      await expect(table.locator('thead th')).toHaveCount(160)
      const pre = workbench.transcript.locator('pre').filter({ hasText: 'xxxx' }).first()
      await expect(pre).toContainText('x'.repeat(220))
      await expect(pre).toHaveCSS('overflow-x', 'auto')

      // The wide table scrolls inside its own entry, not the transcript pane
      // or the document (blueprint 4.1).
      const scrollers = workbench.transcript.locator('.sebas-table-scroll')
      await expect(scrollers).toHaveCount(1)
      const scroller = scrollers.first()
      await expect(scroller.locator('table')).toHaveCount(1)
      await expect(scroller).toHaveCSS('overflow-x', 'auto')
      expect(
        await scroller.evaluate((element) => element.scrollWidth > element.clientWidth),
      ).toBe(true)

      // The document itself must never scroll horizontally; the transcript
      // pane is the only controlled overflow surface.
      const overflow = await page.evaluate(() => {
        const root = document.scrollingElement!
        return { doc: root.scrollWidth - root.clientWidth }
      })
      expect(overflow.doc).toBeLessThanOrEqual(1)

      expect(collector.clean()).toEqual([])
    })

    test('settings is a workspace without a URL and preserves inputs and focus context', async ({
      page,
    }) => {
      await resetState(page.request)
      const { key } = await idleSession(page.request, 'idle settings')
      const shell = new ReactShell(page)
      const settings = new ReactSettings(page)
      const workbench = new ReactWorkbench(page)

      await page.goto(`/sessions/${key}`)
      await expect(workbench.title).toHaveText('idle settings', { timeout: 15_000 })
      await workbench.composerTextarea.fill('unsent draft')

      await shell.openSettings()
      // No resurrected /settings route.
      await expect(page).toHaveURL(new RegExp(`/sessions/${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`))
      await expect(settings.nav.getByRole('button', { name: '通用' })).toBeVisible()
      // RBAC (auth-off = role null): user administration stays hidden.
      await expect(settings.nav.getByRole('button', { name: '用户' })).toHaveCount(0)

      await settings.openSection('外观')
      await settings.openSection('别名')
      // Unsubmitted input survives section switches (no false success/failure).
      await settings.aliasInput.fill('draft-alias')
      await settings.aliasUpstreamInput.fill('draft-upstream')
      await settings.openSection('通用')
      await settings.openSection('别名')
      await expect(settings.aliasInput).toHaveValue('draft-alias')
      await expect(settings.aliasUpstreamInput).toHaveValue('draft-upstream')

      await page.keyboard.press('Escape')
      await expect(settings.region).toBeHidden()
      // Back in the focused session with its unsent input intact.
      await expect(page).toHaveURL(new RegExp(`/sessions/${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`))
      await expect(workbench.composerTextarea).toHaveValue('unsent draft')

      // The retired /settings path redirects to the dashboard instead of
      // opening a phantom settings page.
      await page.goto('/settings')
      await expect(page).toHaveURL(/\/$/)
      await expect(shell.app).toBeVisible()
      await expect(settings.region).toHaveCount(0)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('设置关于与项目路径边界', () => {
    test('about renders instance and build truth from /api/about', async ({ page }) => {
      await resetState(page.request)
      const shell = new ReactShell(page)
      const settings = new ReactSettings(page)

      await page.goto('/')
      await expect(shell.app).toBeVisible()
      await shell.openSettings()
      await settings.openSection('关于')
      const about = settings.region
      await expect(about.getByRole('heading', { name: '关于' })).toBeVisible()
      await expect(about.getByText('版本', { exact: true })).toBeVisible()
      await expect(about.getByText('构建时间', { exact: true })).toBeVisible()
      await expect(about.getByText('默认 agent', { exact: true })).toBeVisible()
      // The panel reports the server truth, not a placeholder error.
      await expect(about.getByText(/加载失败|请求失败/)).toHaveCount(0)

      // Services stays reachable in the auth-off single-machine shape.
      await settings.openSection('服务')
      await expect(about.getByRole('heading', { name: '服务' })).toBeVisible()
      expect(collector.clean()).toEqual([])
    })

    test('manual project path outside the workspace states its reason and blocks submit', async ({
      page,
    }) => {
      await resetState(page.request)
      const shell = new ReactShell(page)
      const rail = new ReactRail(page)

      await page.goto('/')
      await expect(shell.app).toBeVisible()
      await rail.addProjectButton.click()
      const dialog = page.getByRole('dialog', { name: '选择项目目录' })
      await expect(dialog).toBeVisible()
      const input = dialog.getByLabel('项目路径')
      const submit = dialog.getByRole('button', { name: /使用此路径/ })

      // An existing directory outside the sandbox workspace root gets the
      // boundary reason, and submission stays blocked.
      await input.fill('/etc')
      await expect(dialog.getByRole('alert')).toContainText('workspace root 之外', {
        timeout: 10_000,
      })
      await expect(submit).toBeDisabled()

      // A missing path has its own honest reason, still blocked.
      await input.fill(`no-such-dir-${Date.now()}`)
      await expect(dialog.getByRole('alert')).toContainText('路径不存在或无法访问', {
        timeout: 10_000,
      })
      await expect(submit).toBeDisabled()

      await page.keyboard.press('Escape')
      await expect(dialog).toBeHidden()
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('总览与用量', () => {
    test('sessions overview focuses a conversation; usage page keeps its controls', async ({
      page,
    }) => {
      await resetState(page.request)
      const { key } = await idleSession(page.request, 'idle overview')
      const shell = new ReactShell(page)
      const sessionsPage = new ReactSessionsPage(page)
      const usage = new ReactUsagePage(page)

      await page.goto('/sessions')
      await expect(sessionsPage.pageRoot).toBeVisible()
      await expect(sessionsPage.title).toBeVisible()
      await expect(sessionsPage.card('idle overview')).toBeVisible()
      await sessionsPage.focus('idle overview')
      await expect(page).toHaveURL(
        new RegExp(`/sessions/${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`),
      )

      await shell.navButton('用量').click()
      await expect(usage.pageRoot).toBeVisible()
      await expect(usage.sourceGroup).toBeVisible()
      await expect(usage.granularityGroup).toBeVisible()
      await expect(usage.dimensionGroup).toBeVisible()
      await usage.dimensionGroup.locator('.ant-segmented-item-label', { hasText: '输入' }).click()
      await expect(page).toHaveURL(/\/usage$/)

      expect(collector.clean()).toEqual([])
    })
  })
})
