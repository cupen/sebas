## MODIFIED Requirements

### Requirement: Add project via directory picker

The workbench SHALL provide a modal dialog with a server-side directory tree browser and a manual path input, either of which SHALL register a project directory. The tree browser SHALL lazy-load subdirectory listings on expand from `GET /api/fs/browse-dirs?path=...&root=...`, scoped to the server-side work root. The tree SHALL open at the server's default work directory (the listing rooted there, its canonical path displayed so the operator can see the starting scope), and expanding any node SHALL list its subdirectories without error — including on Windows, where previously the path round-trip failed with a 400. The registered project name SHALL be the directory's basename. The system SHALL probe the directory for a git branch after registration and display it in the project row. The manual path input SHALL NOT be bounded by the tree's root scope.

#### Scenario: add project via directory browser

- **WHEN** the operator opens the Add Project dialog and expands a node in the directory tree
- **THEN** the system fetches subdirectory listings from `GET /api/fs/browse-dirs` on demand, presents them lazily, and the operator selects a directory, which registers it as a project

#### Scenario: tree opens at the server work directory

- **WHEN** the Add Project dialog opens
- **THEN** the directory tree's top level lists the contents of the server's default work directory and shows its canonical path, without requiring the operator to navigate down from a filesystem root

#### Scenario: expanding a node lists subdirectories without error

- **WHEN** the operator expands a directory node in the tree on any supported platform (including Windows)
- **THEN** the node's subdirectories are listed beneath it; a failed listing shows an inline error in the tree instead of a silent empty node

#### Scenario: add project via manual path

- **WHEN** the operator types a path into the manual input field and clicks "Add project"
- **THEN** the path is validated and registered, with the same behaviour as the browser path

#### Scenario: manual path outside the tree root still registers

- **WHEN** the operator types a valid directory path that lies outside the tree's starting scope and submits it
- **THEN** the directory is registered as a project

#### Scenario: project name from directory name

- **WHEN** a project is registered at `/home/user/work/my-repo`
- **THEN** the project name is `my-repo`

#### Scenario: git branch shown after registration

- **WHEN** a project is registered and the directory is a git repository
- **THEN** the project row shows the current branch name

#### Scenario: empty directory is not expandable

- **WHEN** a directory has no subdirectories
- **THEN** it has no expand chevron, stays aligned with other rows, and clicking it only selects the path without a loading state
