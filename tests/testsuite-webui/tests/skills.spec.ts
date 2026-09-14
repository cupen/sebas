/**
 * Journey — skills store surface (add-agent-skills 5.2).
 *
 * 功能：Settings 的 Skills 分区 / 子功能：仓列表、SKILL.md 预览、删除、
 * 刷新、sync 结果面板。
 *
 * 数据源是**真文件系统仓**：webui 对 `<scene>/agents-skills` 现扫盘（core
 * `scan_store` 无索引无缓存），所以旅程直接往沙箱仓里种目录——一个 valid
 * （`beads`，frontmatter 齐全 + 1 个嵌套 attachment）一个 invalid（`broken`，
 * 缺 SKILL.md），每个用例开头重种，保证对 retry 与用例顺序都幂等。
 *
 * sync 旅程的 backend 落点（claude → `~/.claude/skills`）由 harness 钉进沙箱
 * （tasks.py `_sandbox_env` 钉 HOME，`[skills] dir` 钉仓）——投影、私产、
 * no_placement 断言全部落在 scene 目录内，绝不触碰操作员真实目录。
 * sandbox 配了两个 agent（claude / fakeacp）：claude 命中方言表，fakeacp 走
 * NoPlacement（spec「reported, not skipped」的如实呈现）。
 */
import fs from 'node:fs'
import path from 'node:path'
import { expect, test } from '@playwright/test'
import { ErrorCollector, SettingsModal, resetState, sceneDir } from './helpers/index'

const BEADS_SKILL = [
  '---',
  'name: beads',
  'description: beads 工作流技能',
  '---',
  '',
  '# beads workflow',
  '',
  'Track tasks with **bd**, never markdown TODO lists.',
  '',
].join('\n')

function storeDir(): string {
  return path.join(sceneDir(), 'agents-skills')
}

/** claude 的 skill 落点（HOME 钉进 scene → `<scene>/.claude/skills`）。 */
function claudePlacementDir(): string {
  return path.join(sceneDir(), '.claude', 'skills')
}

function writeSkill(name: string, body: string | null, attachments: string[] = []): void {
  const dir = path.join(storeDir(), name)
  fs.mkdirSync(dir, { recursive: true })
  if (body !== null) fs.writeFileSync(path.join(dir, 'SKILL.md'), body)
  for (const rel of attachments) {
    const file = path.join(dir, rel)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, 'attachment stub\n')
  }
}

/** 重种两个条目：beads（valid，1 个嵌套 attachment）+ broken（缺 SKILL.md）。 */
function seedStore(): void {
  fs.rmSync(storeDir(), { recursive: true, force: true })
  writeSkill('beads', BEADS_SKILL, ['refs/playbook.md'])
  writeSkill('broken', null)
}

test.describe('Skills 仓', () => {
  let collector: ErrorCollector

  test.beforeEach(({ page }) => {
    collector = new ErrorCollector(page)
  })

  test.describe('列表与预览', () => {
    test('K1 list renders store entries: description, attachment count, invalid badge with reason', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      seedStore()
      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Skills')

      const toolbar = settings.panel.locator('.provider-toolbar span.label')
      await expect(toolbar).toHaveText('2 skills in store', { timeout: 10_000 })

      const rows = settings.panel.locator('[data-testid="skill-row"]')
      await expect(rows).toHaveCount(2)

      // valid 条目：描述 + attachment 计数（嵌套文件也计入）。
      const beads = settings.panel.locator('[data-testid="skill-row"][data-name="beads"]')
      await expect(beads.locator('.skills-row-desc')).toHaveText('beads 工作流技能')
      await expect(beads.locator('.skills-row-atts')).toHaveText('1 attachment')
      await expect(beads.locator('[data-testid="skill-invalid"]')).toHaveCount(0)

      // invalid 条目：徽标 + 悬停原因（title）+ 描述位呈现成因。
      const broken = settings.panel.locator('[data-testid="skill-row"][data-name="broken"]')
      const badge = broken.locator('[data-testid="skill-invalid"]')
      await expect(badge).toBeVisible()
      await expect(badge).toHaveAttribute('title', /SKILL\.md/)
      await expect(broken.locator('.skills-row-desc')).toContainText('SKILL.md')
      await settings.close()

      expect(collector.clean()).toEqual([])
    })

    test('K2 preview lazily renders SKILL.md markdown plus attachment names; invalid entry states the missing body honestly', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      seedStore()
      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Skills')

      // 展开 beads：markdown 渲染（marked 管线）+ attachment 文件名清单。
      const beads = settings.panel.locator('[data-testid="skill-row"][data-name="beads"]')
      await beads.locator('.skills-row-name').click()
      const preview = beads.locator('[data-testid="skill-preview"]')
      await expect(preview.locator('.skills-md h1')).toHaveText('beads workflow', {
        timeout: 10_000,
      })
      await expect(preview.locator('.skills-md strong')).toHaveText('bd')
      await expect(preview.locator('.skills-atts code')).toHaveText('refs/playbook.md')

      // 收起（同名再点）。
      await beads.locator('.skills-row-name').click()
      await expect(preview).toHaveCount(0)

      // invalid 条目没有正文：如实呈现，不冒充空正文。
      const broken = settings.panel.locator('[data-testid="skill-row"][data-name="broken"]')
      await broken.locator('.skills-row-name').click()
      await expect(broken.locator('[data-testid="skill-preview"]')).toContainText(
        'SKILL.md 缺失',
        { timeout: 10_000 },
      )
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('删除与刷新', () => {
    test('K3 delete confirms with the backend-cleanup copy, then refresh reflects the store on disk', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      seedStore()
      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Skills')

      const broken = settings.panel.locator('[data-testid="skill-row"][data-name="broken"]')
      await expect(broken).toBeVisible({ timeout: 10_000 })
      await broken.locator('button[title="Delete"]').click()

      // 确认弹窗点名条目，并讲明「只删仓，backend 副本留待下次 sync 清理」。
      const confirm = page.locator('sebas-settings-modal wa-dialog[label="Delete skill"]')
      const text = confirm.locator('[data-testid="skill-delete-text"]')
      await expect(text).toBeVisible()
      await expect(text).toContainText('Delete skill')
      await expect(text).toContainText('broken')
      await expect(text).toContainText('cleaned up the next')
      await expect(text).toContainText('Sync')

      await confirm.locator('wa-button').filter({ hasText: 'Delete' }).click()
      await expect(confirm).toBeHidden({ timeout: 10_000 })
      await expect(broken).toHaveCount(0)
      await expect(settings.panel.locator('.provider-toolbar span.label')).toHaveText(
        '1 skill in store',
      )
      // API truth：删除只动了仓（fs 事实先行，UI 已随 reloadSkills 对齐）。
      expect(fs.existsSync(path.join(storeDir(), 'broken'))).toBe(false)
      expect(fs.existsSync(path.join(storeDir(), 'beads'))).toBe(true)

      // 刷新后仍与盘上一致（「refresh reflects on-disk community changes」）。
      await settings.panel.locator('wa-button').filter({ hasText: 'Refresh' }).click()
      await expect(settings.panel.locator('[data-testid="skill-row"]')).toHaveCount(1)
      await expect(
        settings.panel.locator('[data-testid="skill-row"][data-name="beads"]'),
      ).toBeVisible()
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })

  test.describe('同步投影', () => {
    test('K4 sync panel reports per-backend counts, keeps private entries, and reports no-placement backends', async ({
      page,
    }) => {
      const settings = new SettingsModal(page)

      // 只种 valid 的 beads；落点预置一个名外私产 + 清掉旧名册，数字确定。
      fs.rmSync(storeDir(), { recursive: true, force: true })
      writeSkill('beads', BEADS_SKILL)
      const placement = claudePlacementDir()
      fs.rmSync(placement, { recursive: true, force: true })
      const priv = path.join(placement, 'user-private')
      fs.mkdirSync(priv, { recursive: true })
      fs.writeFileSync(path.join(priv, 'SKILL.md'), '---\nname: user-private\ndescription: 私产\n---\n')

      await resetState(page.request)
      await page.goto('/')
      await settings.openViaSidebar()
      await settings.openSection('Skills')

      await settings.panel.locator('wa-button').filter({ hasText: 'Sync' }).click()
      const panel = settings.panel.locator('[data-testid="skills-sync-result"]')
      await expect(panel).toBeVisible({ timeout: 10_000 })

      // claude：1 写入 · 私产只计数（不动）。
      const claudeRow = panel.locator('.skills-sync-row', { hasText: 'claude' })
      await expect(claudeRow.locator('.skills-sync-counts')).toHaveText(
        '1 written · 0 overwritten · 0 deleted · 1 private',
      )

      // fakeacp 无落点：如实报告而不是静默跳过。
      await expect(panel.locator('.skills-sync-noplace')).toContainText('no placement: fakeacp')

      // 投影真的落进了沙箱内的落点目录；私产字节不动。
      expect(fs.existsSync(path.join(placement, 'beads', 'SKILL.md'))).toBe(true)
      expect(fs.existsSync(path.join(priv, 'SKILL.md'))).toBe(true)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
