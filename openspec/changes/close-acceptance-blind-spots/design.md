# close-acceptance-blind-spots Design

## Context

事故复盘（2026-09-21）：operator 实例上基本对话失败，沙箱验收全绿。根因是 shell
导出的 `ANTHROPIC_BASE_URL`/`ANTHROPIC_MODEL` 经 `sebas run` 继承进 Claude 子进程，
把请求引向公司网关上未路由的模型名；claude 内部重试约 3 分钟后才以合成错误收尾。
四盲区见 proposal。约束：沙箱规矩（agent 零真实凭据）不破；现有三套测试与覆盖账本
是既成资产，只补洞不重构。

## Goals / Non-Goals

**Goals:**

- 四盲区全部落成 spec requirement + 实现 + 自动化用例
- fake-claude 具备全行为 mock 能力，成为覆盖扩展的唯一模型替身
- 核心命中硬指标 100%，账本基数与工作树 specs 一致
- QA 验收有可分派的技能入口

**Non-Goals:**

- 不改 provider 解析与 cover 语义本身（claude-env-cover 现状不动）
- 不做平行 QA 规范文档；不动 operator 实例

## Decisions

- **D1 env posture 检测放启动路径**（cli-service delta）：core/webui 启动时一次性
  检测 + WARN，与 claude-env-cover 的 cover 判定共用同一解析结果。备选「spawn 点检
  测」被否：每 spawn 一次噪音大，且启动时告警才是 operator 可行动的时点。只 WARN，
  不篡改 env——行为语义归 claude-env-cover 管，这里只补可观测性。
- **D2 重启收敛选重投优先**：恢复时 spawning → 重发 spawn 指令（与 dormant 懒恢复
  语义一致）；重投失败（agent 缺失等）才落合成错误条目转 idle。备选「一律落失败」
  被否：会把可恢复会话误杀。
- **D3 零输出落点用新元素类型 `notice`**：复用 `error` 会触发失败语义（红泡、
  failure_class），零输出不是失败。前端 transcript-view 增加中性信息条渲染分支。
  备选「复用 markdown + 约定文案」被否：语义不可区分，测试没法精确断言。
- **D4 fake-claude 场景集与参数形态**：沿用键值 argv（场景参数无法走位置参数，解析
  期即拒）。场景：`default`（现行行为）/`thinking`/`tool-loop`/`empty`/`slow`/
  `error`，配 `--delay-ms` 全局延迟参数；journal 逐行补记 `scenario` 字段供断言。
  慢响应与停滞看门狗、反馈时限用例共用。
- **D5 硬指标 100% 的可保持性**：豁免不计分母的口径不变，这是 100% 可达的前提；
  「归档时同步重算回填」写进覆盖通过标准，防止账本再次漂移（本次漂移即 2026-09-11
  复核后多轮归档未重算）。
- **D6 QA 技能形态**：`.agents/skills/qa-acceptance/SKILL.md`；询问用结构化选项
  （核心五簇/非核心/全量/快速冒烟）；subagent 以只读 + 既有 invoke 入口执行，避免
  技能本体长跑测试阻塞主会话。快速冒烟 = `invoke testsuite-e2e` 核心子集 + 账本
  头部核对，不跑全量。
- **D7 smoke-real 是 operator 手跑配方**：凭据只经环境变量进入，入口前置校验缺失
  即拒；拓扑照 AGENTS.md 沙箱菜谱钉进一次性目录，跑完即毁。MUST NOT 进 CI / 由
  agent 自动运行——这条写进 spec 是防线不是装饰。

## Risks / Trade-offs

- 100% 硬指标比 90% 更脆：任何核心 requirement 新增而用例未跟上即挡通过。缓解：
  归档同步重算入账；缺口必须当场补测或显式豁免。
- `notice` 元素类型是新的投影面：历史账本中 transcript 相关断言需确认不受影响
  （零输出条目只在空回合出现，正常回合投影不变）。
- fake-claude 场景增多后桩自身成为被测依赖：桩的行为用进程级 e2e 反向断言（场景
  用例即桩的验收），桩坏了套件会红，不会静默假绿。
