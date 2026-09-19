/**
 * fix-webui-approval-restore-and-session-identity 5.4（design D8）：图标资源
 * 本地化的合同——`<wa-icon>` 的 SVG 子集打包进 `public/icons/`（构建期进
 * dist），运行时经 `setIconPath('/icons')` 同源取图，不再从 ka-f.fontaware
 * CDN 拉取（离线 / 受限网络 403 + 破图消除）。
 *
 * 测试直读文件系统断言资产与源码形态（happy-dom 环境下 node:fs 照常可用）；
 * 「断网刷新无 403」的浏览器级验证归 Playwright 回归（change 7.1）。
 */

import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const frontendRoot = join(here, '..', '..')

/** 实际用到的图标子集（folder-picker 的 folder/spinner + rail 的 folder）。 */
const SUBSET: Array<{ file: string; name: string }> = [
  { file: 'solid/folder.svg', name: 'folder' },
  { file: 'regular/folder.svg', name: 'folder' },
  { file: 'regular/spinner.svg', name: 'spinner' },
]

describe('local icon subset (5.4)', () => {
  it('ships every used icon as a same-origin SVG under public/icons', () => {
    for (const { file, name } of SUBSET) {
      const body = readFileSync(join(frontendRoot, 'public', 'icons', file), 'utf8')
      expect(body, `${file} must be an SVG document`).toContain('<svg')
      expect(body, `${file} must carry the ${name} path data`).toContain('<path')
      // xmlns 是命名空间声明不是 fetch 目标；禁的是 CDN 域名。
      expect(body, `${file} must not reference an icon CDN`).not.toMatch(
        /fontawesome\.com|ka-f\.|ka-p\./,
      )
    }
  })

  it('points wa-icon at the local set via setIconPath and drops the CDN', () => {
    const main = readFileSync(join(frontendRoot, 'src', 'main.ts'), 'utf8')
    expect(main).toContain("setIconPath('/icons')")

    // 源码面不再出现 ka-f / fontawesome CDN 引用。
    const offenders: string[] = []
    const walk = (dir: string): void => {
      for (const entry of readdirSync(dir)) {
        const full = join(dir, entry)
        if (statSync(full).isDirectory()) {
          walk(full)
          continue
        }
        if (!/\.(ts|css|html)$/.test(entry)) continue
        const body = readFileSync(full, 'utf8')
        if (/ka-f\.fontawesome\.com|ka-p\.fontawesome\.com/.test(body)) offenders.push(full)
      }
    }
    walk(join(frontendRoot, 'src'))
    walk(join(frontendRoot, 'public'))
    expect(offenders, `CDN references must be gone: ${offenders.join(', ')}`).toEqual([])
  })

  it('the icons route serves the subset from the embedded dist (server side)', () => {
    // webui 静态面挂了 /icons/{*path} → dist 文件（与 /assets 同一嵌入）。
    const serverSrc = readFileSync(
      join(frontendRoot, '..', 'src', 'server.rs'),
      'utf8',
    )
    expect(serverSrc).toContain('"/icons/{*path}"')
  })
})
