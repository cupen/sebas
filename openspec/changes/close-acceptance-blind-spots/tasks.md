# Tasks: close-acceptance-blind-spots

## 1. fake-claude 全行为 mock（后续一切用例的地基）

- [x] 1.1 梳理 fake-claude 现有场景机制与 journal 字段，确定扩展点
- [x] 1.2 实现 `thinking`、`tool-loop`、`empty`、`slow`（`--delay-ms`）、`error` 场景与 `default` 向后兼容；journal 补 `scenario` 字段
- [x] 1.3 进程级 e2e：每场景一条用例（工具环投影序列、空响应→合成提示、慢响应触发停滞自愈、错误响应→error 条目）
- [x] 1.4 单元级验证场景参数解析（键值形态、未知场景报错）

## 2. 盲区 2：env posture 启动告警

- [x] 2.1 实现共享检测：给定 provider 解析结果与进程 env，产出「继承且未覆盖」变量清单
- [x] 2.2 core 与 webui 启动路径接线，WARN 逐变量点名；covering 模式全覆盖时静默
- [x] 2.3 用例：未覆盖继承触发告警、全覆盖静默、检测不改变 env 传递（子进程 env 断言）

## 3. 盲区 4：重启 spawning 收敛

- [x] 3.1 恢复路径实现：spawning → 重投 spawn 指令；重投失败 → 合成错误条目 + idle 终态
- [x] 3.2 单测：落定语义、互不阻塞；e2e：kill 于 spawning 相位 → 重启后无 spawning 会话
- [x] 3.3 顺带核对 dispatch 状态文件中历史 spawning 记录的迁移行为

## 4. 盲区 3：零输出落点 + 提交反馈时限

- [x] 4.1 后端：空回合检测 → 投影追加 `notice` 合成条目（正常回合不受影响）
- [x] 4.2 前端：transcript-view 渲染 `notice` 中性信息条；dark/light 两态
- [x] 4.3 用例：e2e 空响应场景断言 notice 条目；浏览器用例断言提交后 5s 内可见反馈（慢后端场景走 `slow` 桩）
- [x] 4.4 复核 `/code-review` 零回音路径：确认经 4.1 落点后时间线不再静默

## 5. 盲区 1：真实环境冒烟配方

- [x] 5.1 `invoke smoke-real`：env 凭据前置校验（缺失即拒 + 指引）、一次性沙箱拓扑、真实回合一次、env posture 结论打印、退出清理
- [x] 5.2 文档：AGENTS.md 冒烟小节（operator 手跑定位、不进 CI、agent 禁触）
- [x] 5.3 用哑凭据 + fake-provider 演练配方自身逻辑（不拨真上游），确认校验与清理路径

## 6. 账本重算与 100% 达标

- [ ] 6.1 按工作树 specs 重算五簇基数（含本 change 新增 requirement），逐条核对命中证据
- [ ] 6.2 缺口补测：优先用 1.x 场景扩 e2e/浏览器旅程；测不了的面显式豁免注明 cause
- [ ] 6.3 回填 COVERAGE.md：新复核记录、基数差异说明、核心合计 =100%
- [ ] 6.4 testsuite-acceptance 旅程用例按新 requirement 增补（每簇 ≥1 旅程用例保持）

## 7. QA 验收技能与 subagent

- [x] 7.1 `.agents/skills/qa-acceptance/SKILL.md`：范围询问（核心五簇/非核心/全量/快速冒烟）、subagent 派发契约、汇报格式、沙箱红线
- [x] 7.2 subagent 执行说明（只读 + 既有 invoke 入口 + 账本复核命令清单）
- [ ] 7.3 自测一轮：模拟「核心五簇」选择走通询问→派发→汇报闭环

## 8. 收口

- [ ] 8.1 全量测试门：`cargo test`、`invoke testsuite-e2e`、`invoke testsuite-acceptance`、Playwright 全绿
- [ ] 8.2 `openspec validate` 通过；spec delta 与实现一致性走查
- [ ] 8.3 僵尸 spawning 顺带核对：operator 实例下次重启后无 spawning 残留（汇报项，不碰实例）
