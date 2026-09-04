# session-lifecycle Specification

## Purpose

Owns the mapping between Feishu conversations and agent sessions: per-thread session identity, lazy spawn on first message, race-safe spawn bookkeeping, dormant sessions restored across daemon restarts, turn queuing with back-pressure, and the full cleanup contract when a session dies.

## MODIFIED Requirements

### Requirement: Session identity is per chat and thread

The system SHALL key session mappings by the channel-neutral session identity
(`ChannelKey`: channel name plus channel-specific opaque reference, see
`channels` and `openspec/glossary.md`). For the feishu channel the opaque
reference continues to distinguish chat and topic, so a chat holding multiple
topics still holds multiple independent mappings. Web UI sessions continue to
use synthetic references with no thread component, now under the `web` channel.

#### Scenario: Two topics in one chat are separate sessions

- **WHEN** messages arrive from two different topics in the same chat
- **THEN** each topic maps to its own session with its own conversation history

#### Scenario: Main chat maps independently of topics

- **WHEN** a message arrives in the chat's main thread while topics exist in the same chat
- **THEN** the main thread maps to its own session, separate from any topic session