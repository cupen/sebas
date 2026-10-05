// @vitest-environment jsdom
/**
 * 文案快照防回归（webui-i18n-sweep 3.1）：zh 基准的文案合同。
 *
 * 仓库既有两类断言惯例——组件渲染断言与 a11y.test.ts 的「源码含关键串」
 * 断言。本套件沿用后者：对 design.md 清扫清单逐项快照（旧英文样本 SHALL
 * NOT 再出现、对应中文文案 SHALL 在位），再对错误呈现层做模式抽查（错误
 * 文本一律走 errorText，禁止裸 String(err) 拼出英文 "Error: " 前缀）。
 *
 * 断言对象是源码而非运行时 DOM：文案常量在模板字面量里，源码断言与
 * a11y.test.ts 同款，省去逐组件挂载的成本；真渲染行为由各组件自己的
 * *.test.ts 覆盖（本套件改到的断言已同步）。
 */
import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
/** 读源码并剥掉注释（块注释 + 行注释）：断言对象是文案本身，不是解说词。 */
const read = (rel: string): string =>
  readFileSync(join(here, rel), 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '')

/** design.md 清扫清单逐项：文件 → [旧英文样本（不得再现）, 新中文文案（必须在位）]。 */
const SWEEP: ReadonlyArray<[string, ReadonlyArray<[string, string]>]> = [
  [
    'views/sessions.ts',
    [
      ['Closing will terminate the agent child process', '关闭将终止 agent 子进程'],
      ['>Sessions</h1>', '>会话</h1>'],
      ['Nothing running yet', '还没有运行中的会话'],
      ['>New session</wa-button', '>新建会话</wa-button'],
      ['>Cancel</wa-button', '>取消</wa-button'],
      ['>Close</wa-button', '>关闭</wa-button'],
      ['>Focus</wa-button', '>聚焦</wa-button'],
    ],
  ],
  [
    'views/settings-modal.ts',
    [
      ['No general preferences yet', '暂无通用偏好'],
      ['No providers configured.', '尚未配置 provider。'],
      ["'no default set'", "'未设置默认'"],
      ['Delete provider', '删除 provider'],
      ['This removes it from the router', '这会把它从 router 配置中移除'],
      ['>Variable</th>', '>变量</th>'],
      ['>Used for</th>', '>用途</th>'],
      ['>Value</th>', '>值</th>'],
      ['＋ New (preset)', '＋ 新建（预设）'],
      ['＋ New (custom)', '＋ 新建（自定义）'],
      ['＋ New agent', '＋ 新建 agent'],
      ['>Refresh</wa-button>', '>刷新</wa-button>'],
      ["'Saving…' : 'Save'", "'保存中…' : '保存'"],
      ['Fetch model list from', '从 provider 的官方 base URL 抓取模型列表'],
      ['written} · ${r.overwritten.length} overwritten', '已写入 ${r.written.length} · 覆盖 ${r.overwritten.length}'],
      ['no placement: ${o.no_placement', '无落点：${o.no_placement'],
      ['skills in store', '仓内 ${list.length} 个技能'],
      ['The skill store is empty', '技能仓为空'],
      ['Your OS currently asks for', '系统当前为'],
      ['Applied immediately, saved for this browser', '立即生效，保存在当前浏览器'],
      ['>Instance</h3>', '>实例</h3>'],
      ['>Build</h3>', '>构建</h3>'],
      ['>Workspace root</dt>', '>工作区根目录</dt>'],
      ['No users yet.', '还没有用户。'],
      ['label="New user"', 'label="新建用户"'],
      ['Delete skill', '删除技能'],
      ['aria-label="Settings sections"', 'aria-label="设置分区"'],
      ["label: 'Generic'", "label: '通用'"],
      ["label: 'Appearance'", "label: '外观'"],
      ["label: 'Services'", "label: '服务'"],
      ["label: 'Users'", "label: '用户'"],
      ["label: 'Models'", "label: '模型'"],
      ["label: 'Skills'", "label: '技能'"],
      ["label: 'Env Vars'", "label: '环境变量'"],
      ["label: 'About'", "label: '关于'"],
    ],
  ],
  [
    'views/project-rail.ts',
    [
      ['<span>Projects</span>', '<span>项目</span>'],
      ['<span>History</span>', '>历史</a'],
      ['<span>Waiting on you</span>', '<span>等待你处理</span>'],
      ['title="Session actions"', 'title="会话操作"'],
      ['title="Project actions"', 'title="项目操作"'],
      ['`New session in ${p.name}`', '`在 ${p.name} 中新建会话`'],
      ['label="Add project"', 'label="添加项目"'],
      ['Choose a directory to add as a project:', '选择要添加为项目的目录：'],
      ['label="Execution node"', 'label="执行节点"'],
      ['>Add project</wa-button>', '>添加项目</wa-button>'],
      ['<wa-dialog label="Remove project"', '<wa-dialog label="移除项目"'],
      // （fix-webui-qa-round14 5.1，D-5-4）归档 toast 的界面语言一致：
      // 「History」英文残留退役，中文基准文案在位。
      ['可在 History 中查看或恢复', '可在历史中查看或恢复'],
    ],
  ],
  [
    'views/setup-view.ts',
    [
      // （fix-webui-qa-round14 5.1，D-5-4）首启设置页的「Settings」英文残留
      // 退役，与全站「设置」口径一致。
      ['可在 Settings 内管理其他用户', '可在设置内管理其他用户'],
    ],
  ],
  [
    'views/workbench-composer.ts',
    [
      ['Ask for follow-up changes…', '继续对话——描述需要的修改…'],
      ["'core not connected'", "'核心未连接'"],
    ],
  ],
  [
    'views/new-session-dialog.ts',
    [['`New session in ${this.projectName}`', '`在 ${this.projectName} 中新建会话`']],
  ],
  [
    'views/transcript-view.ts',
    [
      ["return 'spawn failed'", "return '启动失败'"],
      ['since you last viewed', '（自你上次查看）'],
      ['mark all seen', '全部标为已读'],
      ['<span class="label">process</span>', '<span class="label">过程</span>'],
    ],
  ],
  [
    'views/dashboard.ts',
    [
      ['title="Focused session"', 'title="聚焦的会话"'],
      ['Agent is immutable — chosen when the session was created', 'agent 创建时选定，之后不可更改'],
      ["'default agent'", "'默认 agent'"],
      ['>last active ${', '>最近活跃 ${'],
      ["return `${diff}s ago`", 'return `${diff} 秒前`'],
    ],
  ],
  [
    'components/review-card.ts',
    [
      ['>Allow once</wa-button', '>仅允许一次</wa-button'],
      ['>Allow for session</wa-button', '>本会话内允许</wa-button'],
      ['>Deny</wa-button', '>拒绝</wa-button'],
      ['>Escalate</wa-button', '>上抛</wa-button'],
      ['No longer pending — already answered', '已不在待决状态'],
      ['aria-label="Permission review"', 'aria-label="权限审批"'],
    ],
  ],
  [
    'app-shell.ts',
    [
      ['aria-label="Sign out"', 'aria-label="退出登录"'],
      ['aria-label="Open settings"', 'aria-label="打开设置"'],
      ['>Settings</span>', '>设置</span>'],
      ['aria-label="sebas console home"', 'aria-label="sebas 控制台首页"'],
    ],
  ],
]

describe('webui-i18n-sweep：zh 基准文案快照（design 清扫清单逐项）', () => {
  for (const [file, pairs] of SWEEP) {
    it(`${file}：旧英文样本退役、中文文案在位`, () => {
      const src = read(file)
      for (const [en, zh] of pairs) {
        expect([file, en, src.includes(en)]).toEqual([file, en, false])
        expect([file, zh, src.includes(zh)]).toEqual([file, zh, true])
      }
    })
  }

  it('原生气泡：登录/首启表单容器 novalidate，缺填走中文自定义校验', () => {
    const login = read('views/login-view.ts')
    expect(login).toContain('<form novalidate')
    expect(login).toContain("return '请输入用户名'")
    expect(login).toContain("return '请输入密码'")
    const setup = read('views/setup-view.ts')
    expect(setup).toContain('<form novalidate')
    expect(setup).toContain("return '请输入用户名'")
    // 浏览器缺省英文气泡的样本（QA 实测）不得回归进这两个表单源码。
    expect(login + setup).not.toContain('Please fill out this field')
  })

  it('错误前缀：视图/组件层错误文本一律 errorText，禁止裸 String(err) 复现 "Error: " 前缀', () => {
    const files = [
      'views/sessions.ts',
      'views/dashboard.ts',
      'views/usage.ts',
      'views/workbench-composer.ts',
      'views/settings-modal.ts',
      'components/review-card.ts',
      'components/pending-stack.ts',
      'components/folder-picker.ts',
    ]
    for (const f of files) {
      const src = read(f)
      expect([f, src]).toContain(f)
      // 裸 String(e)/String(err)（会经 Error.prototype.toString 拼出英文前缀）
      // 在这些消费错误呈现的文件中不得出现（JSON.stringify 等非错误用法不匹配该模式）。
      expect([f, /\bString\((e|err|error)\)/.test(src)]).toEqual([f, false])
      expect([f, src.includes('errorText(')]).toEqual([f, true])
    }
    // 共享实现：client.ts 导出 errorText（Error → message，非 Error 原样字符串化）。
    const client = read('api/client.ts')
    expect(client).toContain('export function errorText(err: unknown): string {')
    expect(client).toContain('err instanceof Error ? err.message : String(err)')
  })
})
