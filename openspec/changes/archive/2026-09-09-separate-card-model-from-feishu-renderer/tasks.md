# Tasks — separate-card-model-from-feishu-renderer

## 1. 语义迁移（已完成）

- [x] 1.1 channels ADDED「Neutral presentation content contract」（1 req / 8 场景）；verify: 通道无关措辞、无 feishu schema/emoji 词汇
- [x] 1.2 feishu-cards delta：REMOVED 8 条中立契约 + MODIFIED 6 条 renderer requirement（含新增「Feishu card layout details」收编 emoji/布局词汇）；verify: validate --changes 通过

## 2. 归档

- [ ] 2.1 `openspec archive separate-card-model-from-feishu-renderer -y`：channels 主 spec 增补契约，feishu-cards 主 spec 收窄；verify: 命令成功、validate --specs 通过、feishu-cards 主 spec 剩 6 条 renderer requirement

## 3. 引用同步

- [ ] 3.1 im-service L38「卡片状态机…遵循 feishu-cards」→ 改指 channels（中立契约）+ feishu-cards（渲染）；verify: 文案含两者
- [ ] 3.2 glossary 卡片词条 `(feishu-cards;channels)` 微调为 `(channels;feishu-cards)` 或加注「中立契约在 channels」；verify: 措辞与 D3 split 一致
