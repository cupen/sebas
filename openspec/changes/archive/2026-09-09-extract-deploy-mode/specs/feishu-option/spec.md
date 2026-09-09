## REMOVED Requirements

### Requirement: webui 主控部署形态
**Reason**: watchdog 的服务启停默认策略（webui 主控、core 默认停、im 跟随通道启用）是部署形态语义，与飞书接入配置无关；随独立 `deploy-mode` capability 迁出。
**Migration**: 见 deploy-mode「Webui-primary deployment shape」。

### Requirement: 双通道共享会话状态
**Reason**: 多通道经 ChannelKey 汇聚到单一会话权威是中立架构语义（不特判任何渠道），随 `deploy-mode` capability 迁出。
**Migration**: 见 deploy-mode「Multi-channel shared session state」。
