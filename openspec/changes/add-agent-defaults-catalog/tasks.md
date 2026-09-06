# Tasks — add-agent-defaults-catalog

## 1. Router admin：defaults 读写面

- [ ] 1.1 `GET/PUT /admin/defaults`：读取/设置默认 provider 与 model，持久化与 providers.json 同域（独立键位），PUT 校验 provider 存在、model 属于其 catalog。验证：router admin 单测（设置/读取/清除/非法 provider 400）。
- [ ] 1.2 删除默认 provider 时联动清除 defaults。验证：单测——删 provider 后 GET defaults 返回未设置。
- [ ] 1.3 控制秘密：无秘密时 PUT 返回 503，GET 放行（与 /admin/providers 同姿态）。验证：单测。

## 2. WebUI BFF 代理

- [ ] 2.1 `GET /api/agent-defaults` 与 `PUT /api/agent-defaults`：代理 router admin defaults（控制秘密），错误状态码透传，鉴权纳入既有 /api/* 门。验证：路由层测试（set→read 回读、无秘密 503）。

## 3. 前端

- [ ] 3.1 provider 管理页：provider 行"设为默认/清除默认"动作 + 当前默认展示；动作调用 BFF 并刷新。验证：settings-modal 组件测试（设默认→展示、清除→无默认态）。
- [ ] 3.2 composer 模型选择器数据源优先级：会话 `available_models` > defaults 指向 provider 的 catalog（无会话时）> 显式不可用提示；catalog/defaults 变更无需建会话即生效。验证：workbench-composer 组件测试（三分支断言）。

## 4. 联调验证（沙箱端到端）

- [ ] 4.1 沙箱起 core+webui（含 router），管理页设默认 provider/model；无会话时打开 composer 模型下拉应列出 catalog 模型。验证：GUI 断言 + 截图。
- [ ] 4.2 清除默认后新工作台下拉呈显式不可用提示；会话存在时下拉回到会话模型列表。验证：GUI 断言。

## 5. 收尾

- [ ] 5.1 全量质量门：`cargo test`、`pnpm -C sebas-webui/frontend test`、`openspec validate add-agent-defaults-catalog --strict`。验证：全绿。
