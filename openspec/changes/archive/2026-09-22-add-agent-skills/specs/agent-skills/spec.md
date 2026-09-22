# agent-skills Specification

## Purpose

Covers the operator-level agent-skill store that sebas owns on the local
machine: a single on-disk repository in agentskills format, the surfaces
used to manage it (CLI + webui), and the manual projection that mirrors
the store into each agent backend's consumption directory under explicit
ownership semantics.

## ADDED Requirements

### Requirement: On-disk skill store in agentskills format

The system SHALL treat one directory (default `~/.agents/skills/`, overridable via config) as the operator-level skill store, where each entry is a skill in the universal agentskills format: a directory named by the skill slug, containing a `SKILL.md` with required frontmatter fields (`name`, `description`) and optionally any number of sibling attachment files.

#### Scenario: List view reads the store directory

- **WHEN** the store directory contains `beads/SKILL.md` and `my-deploy/SKILL.md` (plus attachment files under `my-deploy/`)
- **THEN** the CLI `sebas skills list` and the webui Skills page both report exactly two skills named `beads` and `my-deploy`, with the attachment count visible

#### Scenario: Malformed skill entry surfaces honestly

- **WHEN** the store contains a directory `broken/` whose `SKILL.md` is missing or whose frontmatter lacks `name`/`description`
- **THEN** the listing marks that entry as invalid (with the reason) rather than silently dropping it or aborting the whole list

### Requirement: Add via CLI as a thin wrapper over community flows

`sebas skills add <source>` SHALL place a skill into the store without inventing a private format, where `<source>` is one of: a local directory in agentskills format (validated then copied), a git URL (cloned), or a package spec handed to `npx skills add`. The CLI SHALL NOT fetch over the network on any other command; and after any `add`, the store on disk is the only record (no sebas-maintained index file).

#### Scenario: Add from a local directory

- **WHEN** the user runs `sebas skills add ./my-skill` where `./my-skill` contains a valid `SKILL.md`
- **THEN** the store gains `<store>/my-skill/` with identical contents and `sebas skills list` reports it

#### Scenario: Add rejects a directory that is not a valid skill

- **WHEN** the user runs `sebas skills add ./not-a-skill` where `./not-a-skill` has no `SKILL.md`
- **THEN** the command fails with an error naming the missing `SKILL.md` and the store is unchanged

### Requirement: Webui is a viewing and removal window, not an editor

The webui Settings/Skills page SHALL list the store's skills, render each skill's `SKILL.md` (with attachment file browsing), delete an entry, refresh the listing from disk, and trigger projection. The page SHALL NOT offer create or edit affordances: creation and modification flow through the CLI or community tools (`git`, `npx skills add`), after which the user clicks refresh.

#### Scenario: Refresh reflects on-disk community changes

- **WHEN** a skill has been added to `~/.agents/skills/` by `git clone` outside sebas
- **AND** the user clicks refresh on the Skills page
- **THEN** the newly added skill appears in the listing

#### Scenario: Webui delete removes from the store

- **WHEN** the user deletes skill `beads` on the Skills page
- **THEN** `<store>/beads/` is removed from disk and subsequent listings (webui and CLI) no longer show it

### Requirement: Manual projection with mirror semantics

A sync action (CLI `sebas skills sync` or the webui sync button) SHALL project the store into each configured backend's skill directory. Projection follows mirror-by-name semantics relative to the store: for every skill present in the store, the backend directory receives an identical copy (overwriting any same-named entry that already exists there); for every skill that was previously projected and is now absent from the store, the backend copy is deleted; backend entries whose names do not appear in the store are private and MUST NOT be read, imported, listed, touched, or deleted. Projection never happens automatically — not at session spawn, not on a file watch, not on a timer.

#### Scenario: Sync overwrites a same-named user-modified copy

- **WHEN** the backend directory holds `grill-me/` that the user hand-edited after a previous sync
- **AND** the store's `grill-me/` differs from it
- **THEN** running sync replaces the backend's `grill-me/` with the store's version and reports that one entry was overwritten

#### Scenario: Sync leaves private backend entries untouched

- **WHEN** the backend directory holds `user-byhand/` and the store does not contain `user-byhand`
- **THEN** running sync leaves `user-byhand/` byte-identical and does not list it in the report

#### Scenario: Sync deletes a previously projected entry removed from the store

- **WHEN** the store no longer contains `old-skill` but the backend directory holds a copy whose name matches a skill that was in the store at the last sync
- **THEN** running sync deletes the backend's `old-skill/` and reports the deletion

### Requirement: No projection without a backend placement convention

Each backend kind SHALL either declare a skill-directory convention (where its consumers read skills) or openly refuse projection. A backend with no declared convention is not silently skipped: the sync report MUST show it as having no placement, so the operator does not mistake silence for success. Claude Code's placement SHALL be `~/.claude/skills/`, byte-identical to the store entry (no format conversion).

#### Scenario: Backend without a convention is reported, not skipped

- **WHEN** an agent `gemini` is configured but has no skill-directory convention declared
- **THEN** the sync report contains an entry for `gemini` stating that no placement exists, and no files are written anywhere on its behalf

### Requirement: Local-machine scope only

Projection SHALL target only backend directories on the local machine. Delivery of skills to remote execution nodes over node-link is out of scope for this capability; the store remains visible to remote-node flows only through the pre-existing operator-materials channel (execution-node spec), which this change does not modify.

#### Scenario: Sync with a remote node configured writes nothing remotely

- **WHEN** a remote execution node is configured and connected
- **THEN** `sebas skills sync` writes only to local backend directories and emits no node-link traffic for skills purposes
