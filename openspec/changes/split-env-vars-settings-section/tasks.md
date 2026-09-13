# Tasks: split-env-vars-settings-section

## 1. 后端 /api/env

- [x] 1.1 `sebas-webui/src/api.rs`（+ 路由注册）：新只读端点 `GET /api/env`，静态策划清单（现 UI 表 11 项 + `SEBAS_STATE_DB`），响应 `{items:[{name,what,kind,value}]}`；`std::env::var` 逐项读取；敏感项（PASSWORD/TOKEN/SECRET/FEISHU_APP_SECRET）kind=set_unset 且值不出现在任何字段；非敏感未设置项 value=null 且解释含默认值；经既有鉴权；lib 测试覆盖三分类、遮蔽、未设置标注、清单外变量不出现

## 2. 前端分区迁移

- [x] 2.1 `settings-modal.ts`：分区表增 `env-vars`（底部组 `Env Vars · About`）；Generic 主区换偏好占位文案；环境变量表从 Generic 迁入 env-vars 分区，改消费 `/api/env`（plain 已设置显值、未设置标「未设置（用默认）」、set_unset 显已设置/未设置）；请求失败分区内联错误态；settings-modal.test.ts 补/改用例：分区顺序、Generic 无 env 表、env-vars 渲染三态（值/未设置/敏感）、失败错误态
- [x] 2.2 `pnpm --dir sebas-webui/frontend test` 全绿 + `tsc --noEmit` 零错误

## 3. 收尾

- [ ] 3.1 `cargo test -p sebas-webui` 全绿；`invoke testsuite-webui` 冒烟（settings 相关 journey 若断言旧 Generic env 表需适配）
- [ ] 3.2 `openspec validate split-env-vars-settings-section --strict` 通过
