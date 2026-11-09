# 蓝图与范围评审记录

- 日期：2026-11-08
- 决策：用户选择「批准蓝图，继续实施（推荐）」。
- 批准材料：`interaction-blueprint.md`、独立静态交互预览、`performance-budget.md`、`journey-migration-matrix.md` 与既有基线报告。
- 明确批准范围：全量替换 Lit/Web Awesome、重做核心工作流、完整验收后一次发布，不保留长期新旧界面切换。
- 批准性能预算：`performance-budget.md` 第 2 节硬预算作为实现验收门槛；软预算超出时须记录并评审。
- 基线事实：既有类型检查 45 错，浏览器主 config 148 通过/10 失败/3 跳过，其余 config 未运行。这些不作为本次蓝图通过的对价，也不得在迁移验收中隐去。
- 设计稿验证：独立预览 34 项 Playwright 检查通过、0 console error、四种视口无横向溢出；不代表生产 React 实现验收。
- 后续约束：生产实现须保持既有 HTTP/WS、鉴权/RBAC、深链、流式同步与审批语义；任何范围偏离须重新评审并更新工件。
