## Context

QA 证据（三簇报告 + 截图 + console 记录，`C:\Users\cupen\AppData\Local\Temp\sebas-qa-r10\reports\`）与源码定位：

- 探测链路：`sebas-webui/src/agent_kinds.rs` 的 `resolved_binary` 对含分隔符的路径已做直接文件判定（语义正确）；config 种子行（同一路径）显示「可达」，而 Settings store 行显示 `command not found`——差异只能在 store 行 → `AgentKindSource` 的组装处（`server.rs` 持有 `agent_kinds: Vec<AgentKindSource>`，store 行的 command 构造待查）。
- RBAC：`sebas-webui/src/rbac.rs` 四档矩阵固定（`settings.manage` = root+admin），provider 变更路由（`routes.rs` 的 provider/别名 mutation 端点）未挂既有守卫（对照：`admin_mutation_guard` 已存在于其它 admin 面）。
- 转录渲染：`sebas-webui/frontend/src/views/transcript-view.ts`；C-DEF-01 三次复现的形态（DOM/aria 完好、console 零错误、reload 恢复、超宽表格回合后必现）指向 paint/合成层失效（超宽内容 + containment/overflow 组合的典型病症），而非数据或 DOM 缺失。
- 模型徽章：round9 已落「回合帧模型名」，缺陷是徽章从会话当前模型回填而非帧观察值。
- fakeacp 桩（`fake-acp-agent`）带 `--model-options bad-model,ok-model --reject-model bad-model`，本就是 model-selection 旅程的 oracle。

## Goals / Non-Goals

**Goals:**
- 9 个缺陷逐条修复且各有可复现验收（GUI 旅程优先，单测/API 测试兜底）
- 修复探测误报后解锁 error 场景（上游 5xx 呈现）的补验

**Non-Goals:**
- iab（ZCode 内嵌浏览器）登录兼容性；触屏/i18n；rbac.rs 矩阵重排；error 呈现的新代码（补验除外）

## Decisions

1. **探测修复 = 修 store 行 source 组装，不动 `resolved_binary`**：先写失败单测（store 行 agent + 存在的绝对路径 → `reachable=true`），再修组装处把 store 行的 path 原样放进 `command[0]`（与种子行同构）。被否方案：改 probe 对 `command not found` 的判定加兜底 `is_file` 重查——治标，且会把组装 bug 永久藏住。
2. **不可用文案按 `cause` 分流**：`AgentKindInfo.cause` 已区分缺失原因；前端 dropdown 文案映射改为 cause → 引导面（binary 类 → 「设置 → Agent」，native 凭据类 → 「设置 → 模型」）。不新增协议字段。
3. **dup-id 提示 = 删承诺留拒绝**：表单的「保存将覆盖」warning 仅在编辑既有行时出现（那时它为真）；新建路径遇已存在 id 只显示拒绝 error。前端单测锁两条路径的互斥。
4. **直播态空白 + 超宽撑破同域修**：先写复现测试（宽表回合后流式回合的可见性断言——既有浏览器套件的截图/DOM 断言机制）；修复方向按证据排除法：检查 transcript 容器与超宽条目的 `overflow`/`contain`/`content-visibility` 组合（5350px 隐藏溢出说明容器被内容撑开且 overflow 链条断裂），把横向滚动收敛到条目内层滚动区（`overflow-x: auto` + 可见滚动条样式），容器宽度钉在布局宽。若 CSS 修复后 paint 失效仍在，再查虚拟化/`content-visibility` 的层失效并降级该优化。
5. **模型徽章保真 = 渲染读帧观察值**：条目渲染禁用「会话当前模型」回填路径；帧缺模型名的旧条目显示为无徽章（不伪造）。单测：切模型后旧条目文本不变。
6. **bad-model 拒绝链路调查先行**：任务先做判定实验（浏览器网络面板/console + fakeacp journal 三方对账：composer 选 bad-model 是否发出 `session/set_config_option`）→ 断点在「前端未下发」则补接线（composer 选型 → 现有 switch 端点），在「驱动未投影拒绝」则修驱动错误呈现。spec 场景已钉契约，两条路都收敛到同一验收。
7. **provider 门禁 = 既有守卫接线 + 前端 role-visibility**：服务端在 provider/别名 mutation 路由挂 `settings.manage` 检查（复用 `admin_mutation_guard` 语义）；前端用既有 `role-visibility` 机制隐藏 provider/别名/默认选择的写控件（users/services 已同机制）。读面对所有登录角色保留。
8. **侧栏历史徽章 = 接会话统计源**：会话页统计（活跃/休眠/总数）已有正确数据通道，rail 徽章订阅同源并随 create/close/archive 事件刷新；「历史组 = 归档」语义不变（agent-workbench 既有 Requirement），徽章口径取归档组计数。

## Risks / Trade-offs

- [空白绘制根因若在合成层（浏览器 bug 面），CSS 修复可能不彻底] → 复现测试先行钉住现象；修复分两步走（containment 梳理 → 必要时降级 content-visibility），每步跑三簇 C 的复现路径（B1 会话 + 宽表回合）。
- [store 行组装修复可能牵动 spawn 链路]（可达性与实际 spawn 用同一 command 构造）→ 修复后跑 agent 目录免重启旅程 + fake-claude 会话冒烟，确认「可达」与「可 spawn」同源。
- [provider 门禁收紧可能误伤既有自动化]（脚本用 member token 写 provider）→ 门禁只挂 mutation 写面；acceptance 套件如有 member 写 provider 用例随之改用 admin（搜索确认）。
- [模型徽章历史数据无帧模型名] → 缺失显示为无徽章，不回填伪造；与「诚实呈现」项目口径一致。

## Migration Plan

纯行为修复，无 schema/wire 变更；单 commit 批次随 feat 分支走常规 rebase + --no-ff 合入。回滚 = revert 分支。

## Open Questions

（无——B-DEF-03 的「前端未下发 vs 驱动未投影」是任务内判定实验，两条路的修复都收敛到已钉的 spec 场景，不阻塞拆解。）
