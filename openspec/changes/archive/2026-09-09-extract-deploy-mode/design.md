# Design — extract-deploy-mode

## Context

feishu-option 的三条 requirement 中，仅「Feishu 显式启用开关」是飞书配置语义；另两条（webui 主控部署形态、双通道共享会话状态）是部署与架构语义，恰与 glossary `-option` 补缀（配置开关）定义相悖。参见 proposal.md Why。

## Goals / Non-Goals

- **目标**：两条部署语义迁入新 `deploy-mode`；feishu-option 收窄为纯 `[feishu] enabled` 配置开关；措辞去飞书化（deploy-mode 描述任意 IM 通道而非特判飞书）。
- **非目标**：不改 watchdog 源码行为；不改半配置/env 矩阵；不处理 feishu-* 目录改名（批次 E）。

## Decisions

### 决策 1：新建 deploy-mode 而非并入 watchdog/channels

两条语义分别是「watchdog 默认服务策略」与「多通道会话汇聚」——若前者并入 watchdog、后者并入 channels，会让 watchdog/channels 各背一段与自身主题不同的部署契约；合成 `deploy-mode` 让部署形态有独立归属，也匹配「webui 主控」在 glossary 已是独立词条的现状。

### 决策 2：deploy-mode 措辞渠道中立化

原文写「飞书启用即拉起 im」——deploy-mode 改述为「可选 IM 通道启用即拉起」，把 feishu 降为通道实例而非硬编码；保留原场景文案（Scenario 名中文沿用）以最小化语义漂移。
