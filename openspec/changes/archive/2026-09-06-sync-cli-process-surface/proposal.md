## Why

extract-im-service 落地后(`sebas im` 子命令、`ServiceName::Im` 受管服务、`[watchdog.im]` 配置节),进程与子命令面的三处 spec 未跟上:cli-service 子命令树缺 `im`/`webui-passwd`/`agent-kinds`;watchdog Control request surface 的受管服务列表缺 im;feishu-option 刚归档的场景里还有一处 `sebas watchdog` 旧别名。glossary 的 run 词条监督列表也缺 im。

## What Changes

- `cli-service` Subcommand tree:补入 `im`(独立 IM 服务/飞书 bot 宿主,watchdog 经 `[watchdog.im]` 托管)、`webui-passwd`(WebUI 登录账户管理)、`agent-kinds`(第三方 agent 可达性报告)三个子命令。
- `watchdog` Control request surface:`ServiceSet`/`ServiceRestart` 的受管辅助服务列表 `(webui, router)` → `(webui, router, im)`。
- `feishu-option`:「飞书启用时 im 服务默认拉起」场景中的 `sebas watchdog` → `sebas run`(旧别名已在 fix-spec-gateway-residue 中移除,此为归档 delta 漏网的一处)。
- glossary `run(watchdog 守护)` 词条:监督列表 `core / webui / router` → `core / webui / router / im`(文档,随 apply 直接改)。

## Non-goals

- 不改任何代码与行为——全部是对既有实现的 spec/文档对齐。
- 不动 `status`/`services`/`ctl` 现行别名与 SHALL NOT 拒绝条款。
- 不重写 im-service/channels 等 extract-im-service 刚归档的能力。

## Capabilities

### New Capabilities

(无)

### Modified Capabilities

- `cli-service`:Subcommand tree 需求补三个子命令。
- `watchdog`:Control request surface 需求受管服务列表补 im。
- `feishu-option`:webui 主控部署形态需求一个场景的 THEN 措辞 `sebas watchdog`→`sebas run`。

## Impact

- `openspec/specs/{cli-service,watchdog,feishu-option}/spec.md` + `openspec/glossary.md`;零代码影响。
