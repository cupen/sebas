## MODIFIED Requirements

### Requirement: Webui is a viewing and removal window, not an editor

The webui Settings/Skills page SHALL list the store's skills, render each skill's `SKILL.md` (with attachment file browsing), delete an entry, refresh the listing from disk, and trigger projection. The page SHALL NOT offer create or edit affordances: creation and modification flow through the CLI or community tools (`git`, `npx skills add`), after which the user clicks refresh. Deletion and projection (sync) are store- and backend-mutating actions — sync re-projects the store into the backend placements and removes entries it previously projected that are gone from the store — and SHALL be gated to root/admin (the `settings.manage` permission key, consistent with provider management): the server routes for both delete and sync SHALL reject requests from member/viewer with a typed permission error, and the page SHALL NOT render the delete or sync affordance for read-only roles. The rejection SHALL present the same inline permission notice style the project-registration path uses.

#### Scenario: Refresh reflects on-disk community changes

- **WHEN** a skill has been added to `~/.agents/skills/` by `git clone` outside sebas
- **AND** the user clicks refresh on the Skills page
- **THEN** the newly added skill appears in the listing

#### Scenario: Webui delete removes from the store

- **WHEN** the user deletes skill `beads` on the Skills page
- **THEN** `<store>/beads/` is removed from disk and subsequent listings (webui and CLI) no longer show it

#### Scenario: viewer cannot delete a skill

- **WHEN** a viewer-role user opens the Skills page
- **THEN** no delete affordance is rendered for any skill entry
- **AND** a direct delete request from that session is rejected by the server with a permission error
- **AND** the store directory on disk is unchanged

#### Scenario: member cannot delete a skill

- **WHEN** a member-role user issues a skill delete request
- **THEN** the server rejects it with the same permission error and the store is unchanged

#### Scenario: viewer sees no sync affordance

- **WHEN** a viewer-role user opens the Skills page
- **THEN** the sync control is not rendered alongside the refresh control
- **AND** the refresh (read-only re-listing) control remains available

#### Scenario: viewer cannot trigger projection

- **WHEN** a viewer-role session issues the sync request directly (API replay with the viewer session cookie)
- **THEN** the server rejects it with the same typed permission error
- **AND** no backend placement directory is written or cleaned
