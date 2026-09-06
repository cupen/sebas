## 1. 应用主 specs 对齐

- [x] 1.1 `openspec/specs/cli-service/spec.md`:Subcommand tree 补 `im` / `webui-passwd` / `agent-kinds list`,与 delta 逐字一致;验证:`openspec validate sync-cli-process-surface --strict` 通过
- [x] 1.2 `openspec/specs/watchdog/spec.md`:Control request surface 受管服务列表 `(webui, router)`→`(webui, router, im)`,与 delta 逐字一致
- [x] 1.3 `openspec/specs/feishu-option/spec.md`:「飞书启用时 im 服务默认拉起」场景 `sebas watchdog`→`sebas run`,与 delta 逐字一致
- [x] 1.4 `openspec/glossary.md`:run 词条监督列表 `core / webui / router`→`core / webui / router / im`;验证:`grep -n "sebas watchdog" openspec/specs/ -r` 仅剩 SHALL NOT 拒绝场景

## 2. 验证与归档

- [x] 2.1 `openspec validate sync-cli-process-surface --strict` 通过后归档本 change,确认三处需求场景数不变(Subcommand tree 3、Control request surface 3、webui 主控部署形态 3)
