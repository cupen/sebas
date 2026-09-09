# Tasks — extract-deploy-mode

## 1. 语义迁移（已完成）

- [x] 1.1 deploy-mode ADDED 两条 requirement（Webui-primary deployment shape / Multi-channel shared session state），渠道中立措辞；verify: validate --changes 通过
- [x] 1.2 feishu-option REMOVED 两条部署语义，Migration 指向 deploy-mode；verify: 主 spec 仍保留「Feishu 显式启用开关」

## 2. 归档

- [ ] 2.1 `openspec archive extract-deploy-mode -y`：deploy-mode 主 spec 创建、feishu-option 主 spec 收窄至 1 条；verify: validate --specs 通过、deploy-mode 出现
- [ ] 2.2 feishu-option Purpose 如留 TBD 占位则手工补收窄后表述；verify: Purpose 非占位

## 3. 引用同步

- [ ] 3.1 glossary「主控」词条 `(feishu-option)` → `(deploy-mode)`；verify: grep 无错指
