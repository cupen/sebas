## MODIFIED Requirements

### Requirement: Debug test provider

With `--debug`, the router SHALL inject a built-in `test` provider and
`test → test` route: the router itself answers for all three protocols
(Anthropic, chat-completions, Responses) and both stream modes, skipping
upstream auth and key resolution and the protocol-consistency check. The
model name SHALL carry the scenario: the bare model `test` SHALL keep its
existing echo behavior exactly (fixed text echoing the last user message);
the namespaced forms `test/text`, `test/long`, `test/thinking`,
`test/tool-use`, `test/tools-parallel`, `test/full`, `test/empty`, and
`test/error` SHALL select deterministic response scenarios, each mapped to
core capabilities they exercise. Scenario responses SHALL be composed of
Anthropic content blocks of the kinds the scenario names, and identical
requests SHALL produce identical responses.

For `test/tool-use` and `test/full`, the provider SHALL follow
deterministic agent-loop rules identical in semantics to the fake
upstream's: a request carrying a non-empty `tools` array whose history has
no `tool_result` SHALL be answered with a `tool_use` block naming the
first tool (stop reason `tool_use`, deterministic input); a history that
already contains a `tool_result` SHALL be answered with final text (stop
reason `end_turn`); a request without tools SHALL be answered with plain
text. `test/tools-parallel` SHALL instead answer the first such turn with
a `tool_use` block per declared tool — deterministic distinct inputs — so
parallel tool calls, each with its own permission request, can be driven;
subsequent turns follow the final-text rule. The text-family scenarios
(`text`, `long`, `thinking`, `empty`) SHALL NOT emit tool-use blocks.

`test/long` SHALL produce a long deterministic text body streamed in many
fixed-size chunks. `test/empty` SHALL complete the turn with zero content
blocks. `test/error` SHALL answer with an Anthropic error body (HTTP 5xx,
`api_error`) instead of a message. Streaming responses SHALL use the event
sequence appropriate to each block kind — thinking deltas, input-json
deltas, text deltas — with deterministic chunking, and the final stop
reason SHALL match the scenario. Each message-producing scenario SHALL
report a fixed non-zero token usage (`test` and `test/empty` keep their
existing all-zero usage). On the OpenAI protocol family, the thinking
scenario SHALL degrade to plain text, and `test/error` SHALL produce the
family's error shape.

#### Scenario: debug echo

- **WHEN** the router runs with `--debug` and a request carries model
  `test`
- **THEN** the response is served locally, echoing the request's user
  content, with no upstream contact
- **AND** the response is byte-identical to the pre-scenario behavior

#### Scenario: thinking scenario emits thinking then text

- **WHEN** a request carries model `test/thinking` (non-streaming)
- **THEN** the response content contains a thinking block followed by a
  text block
- **AND** streaming the same request emits thinking deltas before text
  deltas within the correct block events

#### Scenario: tool-use scenario drives the agent loop

- **WHEN** a request carries model `test/tool-use` with a non-empty
  `tools` array and no `tool_result` in history
- **THEN** the response contains a `tool_use` block naming the first
  tool with stop reason `tool_use`
- **AND** when the history already contains a `tool_result`, the response
  is final text with stop reason `end_turn`

#### Scenario: parallel tools scenario emits one tool_use per tool

- **WHEN** a request carries model `test/tools-parallel` with two or more
  declared tools and no `tool_result` in history
- **THEN** the response contains one `tool_use` block per declared tool,
  each with a deterministic distinct input
- **AND** the turn ends with stop reason `tool_use`

#### Scenario: long scenario streams deterministic long text

- **WHEN** a request carries model `test/long` with streaming enabled
- **THEN** the response streams a long deterministic text body in many
  fixed-size text deltas
- **AND** the assembled text is identical to the non-streaming body

#### Scenario: empty scenario completes without content

- **WHEN** a request carries model `test/empty`
- **THEN** the turn completes successfully with zero content blocks and
  stop reason `end_turn`

#### Scenario: error scenario answers with an upstream-style error

- **WHEN** a request carries model `test/error`
- **THEN** the response is an Anthropic error body with HTTP 5xx and type
  `api_error`
- **AND** no message is produced

#### Scenario: scenarios are deterministic

- **WHEN** the same scenario request is sent twice
- **THEN** both responses have identical content blocks, stop reason, and
  usage

#### Scenario: thinking degrades on the OpenAI family

- **WHEN** a `test/thinking` request arrives on the chat-completions
  protocol
- **THEN** the response is plain text
- **AND** no protocol error is produced
