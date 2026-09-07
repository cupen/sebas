## Context

`tests/testsuite-webui/` 套件已覆盖 webui 9 大功能区，但功能骨架显形度不够：spec requirement 同时存在「X 旅程」与「X 核心功能覆盖」两套命名；同一项目增删旅程在 `工作台、项目与会话管理面旅程` 和 `项目管理核心功能覆盖` 下重复挂账；12 个 spec 里只有 `projects.spec.ts` 与 `settings.spec.ts` 走了两层 `test.describe`，其余 10 个把 1–4 个用例摊在一级。COVERAGE.md 的 `testsuite-webui-browser` 段以二维表呈现，看不出父子关系。

本次只调骨架与命名，不改任何用例语义、不动 CI、不动前端被测代码。骨架拉齐后选择器策略与 CI 进保另立 change。

## Goals / Non-Goals

**Goals:**
- spec requirement 统一为「<大功能>覆盖」一种命名（合并去重「X 核心功能覆盖」）。
- 每个 spec 文件维持两层 `test.describe` 结构——顶层对应大功能、二层对应子功能。
- COVERAGE.md `testsuite-webui-browser` 段改写为「大功能 → 子功能 → 用例（spec 文件 / 锚点）」的树形视图。
- `invoke testsuite-webui` 入口加 preflight，挡住在 spec 文件顶层直接 `test(...)` 而绕过两层结构的写法。
- 新加用例只能挂在现有子功能下，命名与结构走齐。

**Non-Goals:**
- 不改任何用例的断言、触发词与选择器；不改前端被测代码。
- 不动 vitest 单测层、不动 CI workflow（CI 进保属于搁置工作）。
- 不解决既有缺口（detached 审批、项目分支端点、watchdog 进程级、record/replay）——继续在 COVERAGE「缺口清单」追踪。
- 不改选择器策略（CSS 深定位 → `getByRole` 收敛另立 change）。

## Decisions

### D1：requirement 命名统一为「<大功能>覆盖」，删除「X 核心功能覆盖」并列写法
- 决策：四条「核心功能覆盖」requirement 合并到对应「X 覆盖」requirement 下的子功能；scenario 全部沿用既有命名（部分新增）。
- 依据：现有 spec 已写「X 旅程」与「X 核心功能覆盖」并立，两套名词指同一类“按功能收敛用例”，读者会以为这是两种不同方法。spec 同时声明“无需改 spec 即可接纳”——意味着渐进补用例是常态，命名不一致会在 COVERAGE 上越积越乱。
- 备选：保留双命名但加注释说明 → 否决：注释不是规约、阅读负担只增不减。

### D2：spec 文件两层 `test.describe`，禁止顶层裸 `test(...)`
- 决策：所有 spec 文件重排为 `test.describe('<大功能>', () => { test.describe('<子功能>', () => { it(...); }); });`；`invoke testsuite-webui` 入口 preflight 用 `rg -nE '^[ ]{0,2}test\('` 在 `tests/testsuite-webui/tests/*.spec.ts` 内检索顶层裸 `test(...)`，命中则拒绝运行并报错。
- 依据：projects/settings 已经自然长出两层 describe 且稳；其他 spec 摊平是因为无外部牵引——给它一个外部规约（preflight）才能避免回弹。preflight 比“code review 检查”可靠：code review 不挡机器，preflight 挡机器。
- 备选：仅靠文档约定 → 否决：projects/settings 的两层结构本身就不在 spec 里写死，靠人遵守的约定不可信。

### D3：scenario 命名沿用 + 拆分，而非新增
- 决策：合并过来的 scenario 全部沿用既有名（如「无模型会话 set_model 诚实拒绝」「流式分批渲染」）；agent 对话下新增「同会话多轮连续」「composer 输入守卫」两个 scenario 收敛现有二期用例。
- 依据：scenarios 是现有用例的索引锚点；改名会让 COVERAGE 上条目与 spec scenario 对不上，需要在 COVERAGE 一并改。
- 备选：所有 scenario 改名以体现子功能 → 否决：账本层改动面大、二期旅程已经按既有 scenario 名断言，重命名无收益。

### D4：COVERAGE.md `testsuite-webui-browser` 段改写为树形表格，不分多张表
- 决策：保留单张表，但列为「大功能 / 子功能 / 用例名 / spec 文件 / 锚点」五列；每行只挂一条用例。
- 依据：树形表格保留二维表的可 diff 性，比真正多张表更容易维护；锚点列直接指向 spec scenario 名，便于回归。
- 备选：拆多张表（一张大功能一张） → 否决：多张表跨大功能时比对成本高，且总行数没省。

### D5：spec 文件头加注释 `> 功能：<requirement 名> / 子功能：<scenario 名>`
- 决策：每个 spec 文件头加三行注释指明归属，避免读者在 COVERAGE 与 spec 之间反复跳。
- 依据：当前 spec 文件头只有 `import { ... }` 与描述，缺归属；COVERAGE 是唯一索引，但 COVERAGE 会被重构，spec 头是稳定锚点。
- 备选：用 `@tags` 等价物 → 否决：Playwright 的 `test.describe` 标题与 `test()` 标题是稳定索引，再用 tag 是双层结构。

### D6：preflight 检查用 ripgrep AST 不可得，用行首缩进正则近似
- 决策：preflight 用 `rg -nE '^[ ]{0,2}test\('`（行首至多两个空格的 `test(`）来近似“顶层裸 test”。豁免规则：若某行紧跟 `test.describe(` 的开括号内允许 `test(`（匹配多行 `test(`）——通过 `rg -U` 关闭即可，不必实现多行解析，由人 review 兜底。
- 依据：spec 文件普遍用 0–4 空格缩进；`test.describe` 块的 `test(...)` 必然在内层有 2 空格以上缩进。误报面极小（只有 export 顶层测试 helper 才可能）；假阳性比假阴性安全（漏报才是骨架回弹的风险）。
- 备选：写一个 TypeScript AST 检查 → 否决：devDeps 缺 `ts-morph`/`@typescript-eslint/parser`，依赖膨胀不划算；骨架规约不需语义级精度。

## Risks / Trade-offs

- [R1] 重排 spec 文件可能引入 flake → mitigation：每个 spec 文件重排后单独跑一次 `invoke testsuite-webui --case <name>` 验证；最后才跑全量。
- [R2] preflight 用行首正则近似“顶层裸 test”，可能误报多行 `test.describe(() => { test(...) })` 写法 → mitigation：错就改缩进（4 空格以内）；不影响语义。
- [R3] COVERAGE.md 改写为树形后行数变多、与 git 历史的可比性变差 → mitigation：commit 分两笔——第一笔改 COVERAGE 结构、第二笔按子功能补用例，差异局部化。
- [R4] 合并四条「X 核心功能覆盖」requirement 时丢文档说明 → mitigation：保留各子功能条款原文到对应 requirement 下方的「子功能」段落，不删字。

## Migration Plan

按以下顺序提交，每笔独立可回滚：

1. **骨架重构（commit 1）**：12 个 spec 文件重排为两层 `test.describe`；spec 文件头加归属注释；spec 文件自身不增加也不删除 it；本 commit 跑 `invoke testsuite-webui` 全量 3 连绿。
2. **账本同步（commit 2）**：COVERAGE.md `testsuite-webui-browser` 段改写为树形；spec requirement 改名 + 合并到 `MODIFIED Requirements`；spec 文件头注释与 COVERAGE 双向校对；本 commit 不需要再跑 e2e（账本与骨架已对齐）。
3. **preflight 上线（commit 3）**：`tasks.py` 的 `testsuite_webui` 加 preflight 函数（ripgrep 检索 + 拒绝运行）；负向用例：在 `tests/testsuite-webui/tests/_probe.spec.ts` 临时加一个顶层裸 `test`，验证 preflight 报错；删 probe 文件后再跑全量 3 连绿。

回滚：commit 1/2 是纯结构/账本改动，git revert 安全；commit 3 的 preflight 可独立 revert。

## Open Questions

无。骨架收敛是纯结构动作，不引入新行为、不引入新依赖、不修改任何被测代码。