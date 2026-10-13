## ADDED Requirements

### Requirement: Agent-select dropdown unavailable label reflects the catalog cause

When an agent catalog row is `reachable: false`, the new-session dialog's agent-select dropdown SHALL render that row as a disabled option with a label that reflects the server-provided `cause` (optionally mapped to operator-friendly phrasing per the "unavailable cause speaks operator language" requirement in `agent-workbench`). The disabled-option label SHALL be derived from the catalog `cause`; the implementation SHALL NOT hardcode a fixed literal that contradicts the server's `cause` for any agent row, and in particular for the `native` row the implementation SHALL NOT hardcode the string "未安装 pi" (or any other fixed text) when the server's `cause` differs. Tooltip text and helper text MAY supplement the primary label, but the primary disabled-option label SHALL be derived from the catalog `cause`.

#### Scenario: native label uses server cause when pi binary is absent

- **WHEN** the catalog's `native` row is `reachable: false` and the server's `cause` reads "pi 二进制不可达 — 到 Settings → Agent 一键安装 pi"
- **THEN** the dropdown renders the `native` option as disabled with a label reflecting that cause (operator-friendly phrasing is allowed but the cause's meaning — binary absent, install surface — SHALL be preserved), not a hardcoded "未安装 pi" or any other fixed string that contradicts the cause

#### Scenario: native label uses server cause when provider is not configured

- **WHEN** pi is installed but no provider is configured and the catalog's `native` row is `reachable: false` with `cause: "未配 provider，请到 Settings → Models 配置"`
- **THEN** the dropdown renders the `native` option as disabled with a label reflecting the missing-provider cause (e.g. "未配 provider — 到 Settings → Models 配置"), not "未安装 pi" — the label and the cause SHALL agree about the actual problem (configuration vs installation)

#### Scenario: non-native row label uses server cause

- **WHEN** any agents-store row (e.g. a `claude` row) is `reachable: false` with a server-provided `cause`
- **THEN** the dropdown renders that row as disabled with a label derived from the cause (e.g. for claude with no provider, "未配 provider — 到 Settings → Models 配置 claude"), not a hardcoded generic string that ignores the cause
