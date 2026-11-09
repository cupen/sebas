/**
 * React 入口的鉴权与 RBAC 旅程（redesign-webui-with-react-antd 5.1）。
 *
 * 只在 `playwright.react-auth.config.ts`（auth-on 沙箱 9892，预置
 * admin/admin）下运行：深链门禁、登录后续跳深链、cookie 跨刷新、登出、
 * 以及只读角色在项目导航与设置分区上的入口裁剪。
 *
 * 错误凭据必须在页面内就地拒绝并显示原因（不依赖 store.error 的副作用）；
 * viewer 的服务分区保留壳，但列表与启停控件按操作级 RBAC 裁剪，并给出
 * 只读说明而不是 403 加载失败。
 */
import { expect, test } from '@playwright/test'
import type { APIRequestContext } from '@playwright/test'
import {
  authLogin,
  createSession,
  ensureSceneProject,
  ErrorCollector,
  ReactLogin,
  ReactRail,
  ReactSettings,
  ReactShell,
  ReactWorkbench,
  waitStatus,
} from './helpers/index'

const VIEWER = 'react-review-viewer'
const VIEWER_PASSWORD = 'viewer-pass'

/** Reuse-safe fixture: drop a leftover viewer from an interrupted run. */
async function ensureViewer(request: APIRequestContext): Promise<void> {
  expect(await authLogin(request, 'admin', 'admin')).toBe(200)
  const existing = (await (await request.get('/api/users')).json()) as {
    users: { id: number; username: string }[]
  }
  for (const user of existing.users.filter((row) => row.username === VIEWER)) {
    await request.delete(`/api/users/${user.id}`)
  }
  const created = await request.post('/api/users', {
    data: { username: VIEWER, password: VIEWER_PASSWORD, role: 'viewer' },
  })
  expect(created.status()).toBe(201)
}

async function removeViewer(request: APIRequestContext): Promise<void> {
  const listed = (await (await request.get('/api/users')).json()) as {
    users: { id: number; username: string }[]
  }
  for (const user of listed.users.filter((row) => row.username === VIEWER)) {
    await request.delete(`/api/users/${user.id}`)
  }
}

test.describe('React 鉴权与 RBAC 旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('深链重定向', () => {
    test('deep link under auth renders the login gate, not the session', async ({ page, request }) => {
      const login = new ReactLogin(page)
      expect(await authLogin(request, 'admin', 'admin')).toBe(200)
      const project = await ensureSceneProject(request)
      const key = await createSession(request, {
        prompt: 'react deep link authed',
        projectId: project.id,
      })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      await expect(login.heading).toBeVisible()
      await expect(page.getByTestId('react-shell')).toHaveCount(0)

      // Logged-out pages surface WS auth-failed noise by design; only real
      // uncaught exceptions must be absent.
      expect(collector.pageErrors).toEqual([])
    })
  })

  test.describe('登录闭环', () => {
    test('wrong credentials are rejected in place with the honest reason', async ({ page }) => {
      const login = new ReactLogin(page)

      await page.goto('/')
      await expect(login.heading).toBeVisible()
      await login.signIn('admin', 'wrong-password')

      await expect(login.error).toHaveText('用户名或密码错误', { timeout: 5_000 })
      await expect(page.getByTestId('react-shell')).toHaveCount(0)
    })

    test('login from a deep link lands on that session', async ({ page, request }) => {
      const login = new ReactLogin(page)
      const workbench = new ReactWorkbench(page)
      expect(await authLogin(request, 'admin', 'admin')).toBe(200)
      const project = await ensureSceneProject(request)
      const key = await createSession(request, {
        prompt: 'react login deep link',
        projectId: project.id,
      })
      await waitStatus(request, key, ['done'])

      await page.goto(`/sessions/${key}`)
      await expect(login.heading).toBeVisible()
      await login.signIn('admin', 'admin')

      await expect(page.getByTestId('react-shell')).toBeVisible({ timeout: 15_000 })
      await expect(page).toHaveURL(new RegExp(`/sessions/${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`))
      await expect(workbench.title).toHaveText('react login deep link', { timeout: 15_000 })
      await expect(workbench.transcript).toContainText('hello')

      expect(collector.pageErrors).toEqual([])
    })

    test('session cookie survives reload until logout, then the gate returns', async ({ page }) => {
      const login = new ReactLogin(page)
      const shell = new ReactShell(page)

      await page.goto('/')
      await expect(login.heading).toBeVisible()
      await login.signIn('admin', 'admin')
      await expect(shell.app).toBeVisible({ timeout: 15_000 })

      await page.reload()
      await expect(shell.app).toBeVisible({ timeout: 15_000 })
      await expect(login.heading).toHaveCount(0)

      await shell.logout()
      await expect(login.heading).toBeVisible({ timeout: 15_000 })
      await page.reload()
      await expect(login.heading).toBeVisible()
      await expect(shell.app).toHaveCount(0)

      expect(collector.pageErrors).toEqual([])
    })
  })

  test.describe('RBAC 入口裁剪', () => {
    test('viewer sees no create entries and no privileged settings sections', async ({
      page,
      request,
    }) => {
      // Reuse-safe fixture: drop a leftover viewer from an interrupted run.
      await ensureViewer(request)

      try {
        const login = new ReactLogin(page)
        const shell = new ReactShell(page)
        const rail = new ReactRail(page)
        const settings = new ReactSettings(page)

        await page.goto('/')
        await expect(login.heading).toBeVisible()
        await login.signIn(VIEWER, VIEWER_PASSWORD)
        await expect(shell.app).toBeVisible({ timeout: 15_000 })

        // Creation entries are hidden for sessions.write=false.
        await expect(rail.addProjectButton).toHaveCount(0)
        await expect(rail.host.getByRole('button', { name: /项目操作/ })).toHaveCount(0)

        // Settings: user administration is root-only; service control and
        // provider/alias writes degrade to read-only for a viewer.
        await shell.openSettings()
        await expect(settings.nav.getByRole('button', { name: '用户' })).toHaveCount(0)
        await expect(settings.nav.getByRole('button', { name: '模型' })).toBeVisible()
        await expect(settings.nav.getByRole('button', { name: '通用' })).toBeVisible()
        await settings.openSection('服务')
        // Operation-level gating: the section shell stays, the service list
        // and controls do not (the read-only wording is covered separately).
        await expect(
          settings.region.getByRole('button', { name: /停\s*用|启\s*用|重\s*启/ }),
        ).toHaveCount(0)
        await settings.openSection('别名')
        await expect(settings.region.getByRole('button', { name: '新建别名' })).toHaveCount(0)

        expect(collector.pageErrors).toEqual([])
      } finally {
        await removeViewer(request)
      }
    })

    test('viewer services section explains read-only access instead of a load error', async ({
      page,
      request,
    }) => {
      // Round-2 defect is fixed: the panel skips the root/admin-only request
      // when the role cannot control services and shows the read-only reason
      // (the server 403 remains the fence).
      await ensureViewer(request)
      try {
        const login = new ReactLogin(page)
        const shell = new ReactShell(page)
        const settings = new ReactSettings(page)

        await page.goto('/')
        await login.signIn(VIEWER, VIEWER_PASSWORD)
        await expect(shell.app).toBeVisible({ timeout: 15_000 })
        await shell.openSettings()
        await settings.openSection('服务')
        await expect(
          settings.region.getByText('服务列表与启停仅 root/admin 可查看'),
        ).toBeVisible()
        await expect(settings.region.getByText(/加载失败/)).toHaveCount(0)
      } finally {
        await removeViewer(request)
      }
    })
  })
})
