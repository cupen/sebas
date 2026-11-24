/**
 * React 入口的窄屏、主题与键盘旅程（redesign-webui-with-react-antd 5.2）。
 *
 * 在 375px 下验证抽屉导航、消息发送与审批都可操作；在深浅主题与
 * prefers-reduced-motion 下验证焦点、状态可辨识且无非必要运动；并横向
 * 检查关键页面不产生意外横向溢出。
 *
 * 两个曾以 `test.fail()` 记录的缺陷已由实现修复并在此保留原断言：
 * 1) 设置窗口关闭后焦点归还「设置」入口；
 * 2) reduced motion 下 AntD Drawer/mask 过渡为 0s（motion 令牌 + CSS）。
 */
import { expect, test } from '@playwright/test'
import {
  createSession,
  cjk,
  ErrorCollector,
  ensureSceneProject,
  ReactRail,
  ReactSettings,
  ReactShell,
  ReactWorkbench,
  resetState,
  waitStatus,
} from './helpers/index'
import type { APIRequestContext, Page } from '@playwright/test'

async function idle(
  request: APIRequestContext,
  prompt: string,
): Promise<{ key: string; project: { id: string; name: string } }> {
  const project = await ensureSceneProject(request)
  const key = await createSession(request, { prompt, projectId: project.id })
  await waitStatus(request, key, ['done'])
  return { key, project }
}

async function docOverflow(page: Page): Promise<number> {
  return page.evaluate(
    () => document.scrollingElement!.scrollWidth - document.scrollingElement!.clientWidth,
  )
}

/** WCAG relative luminance / contrast for the rendered canvas+text pair. */
function contrast(rgbA: string, rgbB: string): number {
  const parse = (rgb: string): [number, number, number] => {
    const match = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(rgb)
    if (match === null) throw new Error(`unexpected colour: ${rgb}`)
    return [Number(match[1]), Number(match[2]), Number(match[3])]
  }
  const luminance = (rgb: string): number => {
    const [r, g, b] = parse(rgb).map((value) => {
      const n = value / 255
      return n <= 0.04045 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4
    }) as [number, number, number]
    return 0.2126 * r + 0.7152 * g + 0.0722 * b
  }
  const a = luminance(rgbA)
  const b = luminance(rgbB)
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)
}

test.describe('React 响应式与可访问性旅程', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('375px 窄屏关键旅程', () => {
    test('drawer navigation switches sessions and preserves unsent input', async ({ page }) => {
      await resetState(page.request)
      const one = await idle(page.request, 'mobile one')
      const two = await idle(page.request, 'mobile two')
      const shell = new ReactShell(page)
      const rail = new ReactRail(page)
      const workbench = new ReactWorkbench(page)

      await page.setViewportSize({ width: 375, height: 812 })
      await page.goto(`/sessions/${one.key}`)
      await expect(workbench.title).toHaveText('mobile one', { timeout: 15_000 })
      // Desktop rail is gone; the toggle is the only navigation entry.
      await expect(shell.rail).toHaveCount(0)
      await expect(shell.narrowMenuToggle).toBeVisible()

      // Unsent input survives opening/closing the drawer.
      await workbench.composerTextarea.fill('narrow draft')
      await shell.openDrawer()
      await expect(shell.drawer).toBeVisible()
      await page.keyboard.press('Escape')
      await expect(shell.drawer).toBeHidden()
      await expect(workbench.composerTextarea).toHaveValue('narrow draft')
      await expect
        .poll(() => page.evaluate(() => document.activeElement?.id ?? ''), { timeout: 5_000 })
        .toBe('nav-menu-toggle')

      // Selecting a session in the drawer closes it and lands on the target.
      await shell.openDrawer()
      // The drawer reuses the rail body but not its desktop <aside> host.
      const drawerRail = new ReactRail(page, shell.drawer)
      await drawerRail.sessionRow('mobile two').click()
      await expect(shell.drawer).toBeHidden()
      await expect(page).toHaveURL(
        new RegExp(`/sessions/${two.key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`),
      )
      await expect(workbench.title).toHaveText('mobile two', { timeout: 15_000 })
      await expect(workbench.transcript).toContainText('hello')
      expect(await docOverflow(page)).toBeLessThanOrEqual(1)

      expect(collector.clean()).toEqual([])
    })

    test('composer and approval stay operable at 375px', async ({ page }) => {
      await resetState(page.request)
      const { key } = await idle(page.request, 'mobile composer')
      const workbench = new ReactWorkbench(page)

      await page.setViewportSize({ width: 375, height: 812 })
      await page.goto(`/sessions/${key}`)
      await expect(workbench.title).toHaveText('mobile composer', { timeout: 15_000 })

      await workbench.send('hello mobile')
      await expect(workbench.transcript).toContainText('hello world', { timeout: 20_000 })
      // Wait for the first turn to settle before triggering the gate: sending
      // into a settling turn must not be what this journey measures.
      await expect(workbench.submitControl).toHaveAttribute('data-state', 'disabled', {
        timeout: 20_000,
      })
      await expect.poll(async () => (await workbench.statusText()), { timeout: 20_000 }).toBe('完成')

      await workbench.send('perm')
      await expect(workbench.operatorUnit('perm')).toBeVisible({ timeout: 15_000 })
      const card = workbench.approvalCard('Bash')
      await expect(card).toBeVisible({ timeout: 20_000 })
      const box = await card.boundingBox()
      expect(box).not.toBeNull()
      // The review card never extends past the 375px viewport.
      expect(box!.x).toBeGreaterThanOrEqual(0)
      expect(box!.x + box!.width).toBeLessThanOrEqual(375)
      await card.getByRole('button', { name: cjk('允许一次') }).click()
      await expect(workbench.approvalCards()).toHaveCount(0, { timeout: 15_000 })
      await expect.poll(async () => (await workbench.statusText()), { timeout: 20_000 }).toBe('完成')
      expect(await docOverflow(page)).toBeLessThanOrEqual(1)

      expect(collector.clean()).toEqual([])
    })

    test('settings opened from the narrow drawer keeps the context', async ({ page }) => {
      await resetState(page.request)
      const { key } = await idle(page.request, 'mobile settings')
      const shell = new ReactShell(page)
      const settings = new ReactSettings(page)

      await page.setViewportSize({ width: 375, height: 812 })
      await page.goto(`/sessions/${key}`)
      await shell.openDrawer()
      await shell.drawer.getByRole('button', { name: /设\s*置/ }).click()
      await expect(settings.region).toBeVisible()
      await expect(shell.drawer).toBeHidden()
      expect(await docOverflow(page)).toBeLessThanOrEqual(1)

      await settings.openSection('外观')
      await page.keyboard.press('Escape')
      await expect(settings.region).toBeHidden()
      expect(await docOverflow(page)).toBeLessThanOrEqual(1)

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('深浅主题与键盘', () => {
    test('theme choice flips tokens, persists and keeps text contrast', async ({ page }) => {
      const shell = new ReactShell(page)
      await page.goto('/')
      await expect(shell.app).toBeVisible()

      const palette = (): Promise<{ theme: string; bg: string; fg: string }> =>
        page.evaluate(() => {
          const app = document.querySelector('[data-testid="react-shell"]') as HTMLElement
          const style = getComputedStyle(app)
          return {
            theme: document.documentElement.dataset.theme ?? '',
            bg: style.backgroundColor,
            fg: style.color,
          }
        })

      const light = await palette()
      expect(light.theme).toBe('light')
      expect(contrast(light.fg, light.bg)).toBeGreaterThanOrEqual(4.5)

      await shell.setTheme('深色')
      const dark = await palette()
      expect(dark.theme).toBe('dark')
      expect(dark.bg).toBe('rgb(6, 8, 12)')
      expect(contrast(dark.fg, dark.bg)).toBeGreaterThanOrEqual(4.5)
      expect(await page.evaluate(() => localStorage.getItem('sebas:theme'))).toBe('dark')

      await page.reload()
      await expect(shell.app).toBeVisible()
      const reloaded = await palette()
      expect(reloaded.theme).toBe('dark')
      expect(contrast(reloaded.fg, reloaded.bg)).toBeGreaterThanOrEqual(4.5)

      expect(collector.clean()).toEqual([])
    })

    test('keyboard focus is visible and opens and closes settings', async ({ page }) => {
      const shell = new ReactShell(page)
      const settings = new ReactSettings(page)
      await page.goto('/')
      await expect(shell.app).toBeVisible()

      // Focus the rail settings entry by keyboard (Tab order includes it).
      await shell.settingsButton.focus()
      await page.keyboard.press('Enter')
      await expect(settings.region).toBeVisible()
      await expect(page).toHaveURL(/\/$/)

      // Focus visible on the active element while working in the workspace.
      const ring = await page.evaluate(() => {
        const style = getComputedStyle(document.activeElement as Element)
        return { style: style.outlineStyle, width: style.outlineWidth }
      })
      expect(ring.style).not.toBe('none')
      // Blueprint: one 2px accent ring across product chrome and AntD controls.
      expect(parseFloat(ring.width)).toBeGreaterThanOrEqual(2)

      await page.keyboard.press('Escape')
      await expect(settings.region).toBeHidden()
      expect(collector.clean()).toEqual([])
    })

    test('closing settings returns focus to the settings trigger', async ({ page }) => {
      const shell = new ReactShell(page)
      const settings = new ReactSettings(page)
      await page.goto('/')
      await expect(shell.app).toBeVisible()

      await shell.openSettings()
      await settings.openSection('外观')
      await page.keyboard.press('Escape')
      await expect(settings.region).toBeHidden()

      await expect
        .poll(() => page.evaluate(() => document.activeElement?.textContent?.trim() ?? ''))
        .toBe('设置')
    })
  })

  test.describe('导航轨尺寸与浮层', () => {
    test('rail resize by keyboard persists across reload', async ({ page }) => {
      await resetState(page.request)
      const shell = new ReactShell(page)
      await page.setViewportSize({ width: 1440, height: 900 })
      await page.goto('/')
      await expect(shell.rail).toBeVisible()

      const handle = page.getByRole('separator', { name: '调整导航宽度' })
      await expect(handle).toHaveAttribute('aria-valuenow', '280')
      await handle.focus()
      await page.keyboard.press('ArrowRight')
      await expect(handle).toHaveAttribute('aria-valuenow', '296')
      await page.keyboard.press('ArrowRight')
      await expect(handle).toHaveAttribute('aria-valuenow', '312')
      await expect
        .poll(async () => Math.round((await shell.rail.boundingBox())?.width ?? 0))
        .toBe(312)

      // Pointer drag uses capture and persists the same way (240–360px).
      const box = await handle.boundingBox()
      expect(box).not.toBeNull()
      const midY = box!.y + 200
      const midX = box!.x + box!.width / 2
      await page.mouse.move(midX, midY)
      await page.mouse.down()
      await page.mouse.move(midX + 40, midY, { steps: 5 })
      await page.mouse.up()
      await expect(handle).toHaveAttribute('aria-valuenow', '352')
      await expect
        .poll(async () => Math.round((await shell.rail.boundingBox())?.width ?? 0))
        .toBe(352)

      await page.reload()
      await expect(shell.rail).toBeVisible()
      await expect(page.getByRole('separator', { name: '调整导航宽度' })).toHaveAttribute(
        'aria-valuenow',
        '352',
      )
      await expect
        .poll(async () => Math.round((await shell.rail.boundingBox())?.width ?? 0))
        .toBe(352)
      expect(collector.clean()).toEqual([])
    })

    test('768 rail is a 56px icon gutter whose labels fly out without reflow', async ({
      page,
    }) => {
      await resetState(page.request)
      const shell = new ReactShell(page)
      await page.setViewportSize({ width: 768, height: 900 })
      await page.goto('/')
      await expect(shell.rail).toBeVisible()
      // Playwright's pointer starts at (0,0), i.e. over the rail; move it to
      // the content first so the measured state is the resting icon gutter.
      await page.mouse.move(760, 800)

      const width = async (): Promise<number> =>
        Math.round((await shell.rail.boundingBox())?.width ?? 0)
      await expect.poll(width).toBe(56)
      const main = page.locator('main').first()
      const gutter = (await main.boundingBox())?.x ?? 0
      expect(gutter).toBeGreaterThanOrEqual(56)

      // Pointer hover opens a temporary overlay; the transcript column keeps
      // its 56px gutter (no reflow) and the page never gains a scrollbar.
      await shell.rail.hover()
      await expect.poll(width).toBeGreaterThan(200)
      expect((await main.boundingBox())?.x ?? 0).toBe(gutter)
      expect(await docOverflow(page)).toBeLessThanOrEqual(1)
      await page.mouse.move(760, 400)
      await expect.poll(width).toBe(56)

      // Keyboard focus opens the same flyout and blur collapses it.
      await shell.settingsButton.focus()
      await expect.poll(width).toBeGreaterThan(200)
      await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
      await expect.poll(width).toBe(56)
      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('减少动态效果', () => {
    test.use({ reducedMotion: 'reduce' })

    test('reduced motion suppresses drawer animation', async ({ page }) => {
      const shell = new ReactShell(page)
      await page.setViewportSize({ width: 375, height: 812 })
      await page.goto('/')
      await expect(shell.app).toBeVisible()
      await expect(page.locator('[data-reduced-motion]')).toHaveAttribute(
        'data-reduced-motion',
        'true',
      )

      await shell.openDrawer()
      const durations = await page.evaluate(() => {
        const content = document.querySelector('.ant-drawer-content-wrapper')
        const mask = document.querySelector('.ant-drawer-mask')
        return {
          content: content ? getComputedStyle(content).transitionDuration : null,
          mask: mask ? getComputedStyle(mask).transitionDuration : null,
        }
      })
      expect(durations.content).toBe('0s')
      expect(durations.mask).toBe('0s')
    })
  })

  test.describe('横向溢出矩阵', () => {
    test('key pages never scroll horizontally at 375/768/1024/1440', async ({ page }) => {
      await resetState(page.request)
      await idle(page.request, 'overflow probe')
      const shell = new ReactShell(page)

      for (const width of [375, 768, 1024, 1440]) {
        await page.setViewportSize({ width, height: 800 })
        for (const path of ['/', '/sessions', '/usage']) {
          await page.goto(path)
          await expect(shell.app).toBeVisible()
          expect(await docOverflow(page), `${path} @ ${width}px`).toBeLessThanOrEqual(1)
        }
        // Settings workspace at every width (drawer entry below 600px).
        if (width >= 600) {
          await page.goto('/')
          await shell.openSettings()
        } else {
          await page.goto('/')
          await shell.openDrawer()
          await shell.drawer.getByRole('button', { name: /设\s*置/ }).click()
        }
        await expect(page.getByRole('region', { name: '设置' })).toBeVisible()
        expect(await docOverflow(page), `settings @ ${width}px`).toBeLessThanOrEqual(1)
        await page.keyboard.press('Escape')
      }

      expect(collector.clean()).toEqual([])
    })
  })
})
