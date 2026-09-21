## ADDED Requirements

### Requirement: Submission acknowledgment is bounded

用户在任一提交面（composer 文本、斜杠命令、显式激活）提交后，该提交面 SHALL 在
5 秒内呈现可见反馈——排队指示、pending 堆栈条目或回合开始帧三者任一。超过 5 秒
无任何可见反馈即违反本要求，无论后端最终是否处理该提交。反馈允许前端乐观呈现
（本地先显示排队，后端确认后再对齐）。

#### Scenario: 提交即时可见

- **WHEN** 用户在 composer 提交一条消息
- **THEN** 5 秒内提交面出现可见的已接收/排队指示

#### Scenario: 慢后端不吞反馈

- **WHEN** 核心通道对提交的确认延迟超过 5 秒
- **THEN** 提交面仍在 5 秒内呈现本地排队指示（乐观呈现即满足），后端确认后对齐
