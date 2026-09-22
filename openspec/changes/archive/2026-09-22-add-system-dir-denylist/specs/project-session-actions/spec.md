## MODIFIED Requirements

### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory tree browser and a manual path input, either of which SHALL register a project directory. The tree browser SHALL lazy-load subdirectory listings on expand from `GET /api/fs/browse-dirs?path=...&root=...`, scoped to the workspace root. The tree SHALL open at the workspace root (the listing rooted there, its canonical path displayed so the operator can see the starting scope), and expanding any node SHALL list its subdirectories without error — including on Windows, where previously the path round-trip failed with a 400. The tree SHALL NOT offer a directory that resolves onto the built-in system-directory denylist. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration to determine directory accessibility; the branch name SHALL NOT be displayed in the project row. The manual path input SHALL be bounded by the workspace root: a path that resolves outside it SHALL be rejected with an out-of-scope error, identically to the browser path. A submitted path that resolves onto a built-in system directory SHALL likewise be refused — through either entry — with an error naming the submitted path shown inline in the dialog.

#### Scenario: add project via directory browser

- **WHEN** the operator opens the Add Project dialog and expands a node in the directory tree
- **THEN** the system fetches subdirectory listings from `GET /api/fs/browse-dirs` on demand, presents them lazily, and the operator selects a directory, which registers it as a project

#### Scenario: tree opens at the server work directory

- **WHEN** the Add Project dialog opens
- **THEN** the directory tree's top level lists the contents of the workspace root (replacing the former server work directory start) and shows its canonical path, without requiring the operator to navigate down from a filesystem root

#### Scenario: expanding a node lists subdirectories without error

- **WHEN** the operator expands a directory node in the tree on any supported platform (including Windows)
- **THEN** the node's subdirectories are listed beneath it; a failed listing shows an inline error in the tree instead of a silent empty node

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: manual path outside the tree root still registers

- **WHEN** the operator types a valid directory path that resolves outside the workspace root and submits it
- **THEN** the registration is rejected with an out-of-scope error and no project is created — the former out-of-scope tolerance is revoked by this change

#### Scenario: system directory via manual input is rejected

- **WHEN** the operator types a path that resolves onto a built-in system directory (for example `/usr`, or `C:\Windows`) and submits it
- **THEN** the registration is rejected, the dialog shows the typed error naming the submitted path, and no project is created

#### Scenario: the tree does not offer denylisted directories

- **WHEN** a browsed listing would contain a directory that resolves onto the built-in denylist
- **THEN** that directory does not appear as a selectable tree node, so it cannot be picked

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

#### Scenario: git branch shown after registration

- **WHEN** a project is registered and the directory is a git repository
- **THEN** the project row does not show a branch name, while the probe result still drives the directory-accessibility marking

#### Scenario: empty directory is not expandable

- **WHEN** a directory has no subdirectories
- **THEN** it has no expand chevron, stays aligned with other rows, and clicking it only selects the path without a loading state
