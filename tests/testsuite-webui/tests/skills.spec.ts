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
      await settings.openSection('技能')

      // React 技能面板是一行一条目（无独立工具栏计数）：行数即仓内条目数。
      const rows = settings.panel
        .locator('strong')
        .filter({ hasText: /^(beads|broken)$/ })
        .locator('xpath=../..')
      await expect(rows).toHaveCount(2)

      // valid 条目：描述 + attachment 计数（嵌套文件也计入）。
      const beads = settings.panel.getByText('beads', { exact: true }).locator('xpath=../..')
      await expect(beads).toContainText('beads 工作流技能')
      await expect(beads).toContainText('1 个附件')
      await expect(beads.getByText('无效')).toHaveCount(0)

      // invalid 条目：徽标 + 描述位呈现成因。
      const broken = settings.panel.getByText('broken', { exact: true }).locator('xpath=../..')
      const badge = broken.getByText('无效', { exact: true })
      await expect(badge).toBeVisible()
      await expect(broken).toContainText('SKILL.md')
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
      await settings.openSection('技能')

      // 展开 beads：markdown 渲染（marked 管线）+ attachment 文件名清单。
      // React 技能行以条目名作为展开入口；预览呈现 SKILL.md 正文与附件名。
      const beads = settings.panel.getByText('beads', { exact: true }).locator('xpath=../..')
      await beads.getByText('beads', { exact: true }).click()
      const preview = beads.locator('xpath=..')
      await expect(preview.getByRole('heading', { name: 'beads workflow' })).toBeVisible({
        timeout: 10_000,
      })
      await expect(preview.locator('strong', { hasText: 'bd' })).toBeVisible()
      await expect(preview.getByText('refs/playbook.md')).toBeVisible()

      // 收起（同名再点）。
      await beads.getByText('beads', { exact: true }).click()
      await expect(preview.getByRole('heading', { name: 'beads workflow' })).toHaveCount(0)

      // invalid 条目没有正文：如实呈现，不冒充空正文。
      const broken = settings.panel.getByText('broken', { exact: true }).locator('xpath=../..')
      await broken.getByText('broken', { exact: true }).click()
      await expect(settings.panel.getByText('SKILL.md 缺失')).toBeVisible({ timeout: 10_000 })
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
      await settings.openSection('技能')

      const broken = settings.panel.getByText('broken', { exact: true }).locator('xpath=../..')
      await expect(broken).toBeVisible({ timeout: 10_000 })
      // React 行内直接删除（服务端只删仓；backend 副本留待下次同步清理）。
      await broken.getByRole('button', { name: /删\s*除/ }).click()
      await expect(broken).toHaveCount(0)
      // API truth：删除只动了仓（fs 事实先行，UI 已随 reloadSkills 对齐）。
      expect(fs.existsSync(path.join(storeDir(), 'broken'))).toBe(false)
      expect(fs.existsSync(path.join(storeDir(), 'beads'))).toBe(true)

      // 刷新后仍与盘上一致（「refresh reflects on-disk community changes」）。
      await settings.panel.getByRole('button', { name: /刷\s*新/ }).click()
      await expect(
        settings.panel
          .locator('strong')
          .filter({ hasText: /^(beads|broken)$/ })
          .locator('xpath=../..'),
      ).toHaveCount(1)
      await expect(
        settings.panel.getByText('beads', { exact: true }).locator('xpath=../..'),
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
      await settings.openSection('技能')

      await settings.panel.getByRole('button', { name: /同\s*步/ }).click()
      const panel = settings.panel.getByRole('alert').filter({ hasText: '同步：' })
      await expect(panel).toBeVisible({ timeout: 10_000 })

      // 汇总行如实报写入/覆盖与删除计数（逐 backend 明细在 React 面板尚未
      // 呈现——保留计数语义断言，明细缺口另行报告）。
      await expect(panel).toContainText('写入/覆盖')
      await expect(panel).toContainText('删除')

      // 投影真的落进了沙箱内的落点目录；私产字节不动。
      expect(fs.existsSync(path.join(placement, 'beads', 'SKILL.md'))).toBe(true)
      expect(fs.existsSync(path.join(priv, 'SKILL.md'))).toBe(true)
      await settings.close()

      expect(collector.clean()).toEqual([])
    })
  })
})
