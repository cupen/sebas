## ADDED Requirements

### Requirement: Directory listing omits system directories

`GET /api/fs/browse-dirs` SHALL NOT list child directories that resolve onto the built-in system-directory denylist. The filter is a coarse UX filter for the picker tree; authoritative enforcement remains at project registration. Non-denylisted entries SHALL be unaffected, and the round-trip contract (an echoed `path` is accepted verbatim as a later request) SHALL continue to hold.

#### Scenario: denylisted child is not listed

- **WHEN** a directory listing would contain a child directory that resolves onto the built-in denylist (for example browsing a workspace root of `/`, which contains `/usr` and `/etc`)
- **THEN** those children are absent from the response entries

#### Scenario: other entries and the round-trip are unaffected

- **WHEN** a directory listing contains only non-denylisted subdirectories
- **THEN** all of them are listed as before, and joining a child name onto the echoed `path` resolves to that child on a subsequent request

### Requirement: Project registration rejects system directories

Local project registration (`POST /api/projects`) SHALL reject a submitted path that resolves onto a built-in system directory with a 400 error naming the submitted path (never the server-resolved form), and no project SHALL be created. The comparison SHALL run on the resolved real path — so `..` segments, path aliases, and symlinks pointing at a denylisted directory are caught — and SHALL be an exact match on the directory itself: a subdirectory of a denylisted directory remains registrable subject to the workspace-root containment rules. On Windows the comparison SHALL be case-insensitive, and drive roots (`C:\`, `D:\`, …) SHALL be matched by pattern rather than enumeration. The built-in denylist SHALL cover, on Unix: `/`, `/bin`, `/sbin`, `/boot`, `/dev`, `/etc`, `/lib`, `/lib32`, `/lib64`, `/libx32`, `/proc`, `/sys`, `/usr`, `/var`, `/run`, `/root`, `/home`, `/tmp` — and deliberately not `/opt`, `/srv`, `/mnt`, `/media`; on Windows: drive roots, `C:\Windows`, `C:\Program Files`, `C:\Program Files (x86)`, `C:\ProgramData`, `C:\Users`, `System Volume Information`, and `$Recycle.Bin`. The denylist is built-in and fixed; no configuration can extend or trim it. Both sides of the comparison SHALL be resolved before matching, so platform directory aliases (for example macOS `/tmp` → `/private/tmp`) hit the list. The denylist judgement SHALL run after the workspace-root containment judgement, which keeps its existing precedence and error.

#### Scenario: system directory itself is rejected

- **WHEN** the workspace root is `/` and `POST /api/projects` is called with path `/usr`
- **THEN** the response is 400 naming `/usr` as a system directory, and no project is registered

#### Scenario: a subdirectory of a denylisted directory stays registrable

- **WHEN** the workspace root is `/` and `POST /api/projects` is called with path `/home/user/code`
- **THEN** the project registers as before — only the denylisted directory itself is refused

#### Scenario: alias and traversal resolve before the comparison

- **WHEN** a submitted path reaches a denylisted directory through `..` segments, a filesystem alias, or a symlink inside the workspace root
- **THEN** the registration is rejected exactly as if the system directory had been named directly

#### Scenario: Windows comparison is case-insensitive and covers drive roots

- **WHEN** on Windows `POST /api/projects` is called with `c:\WINDOWS` or with a drive root such as `D:\`
- **THEN** the response is 400 naming the submitted path, and no project is registered

#### Scenario: out-of-scope precedence is unchanged

- **WHEN** a submitted path lies outside the workspace root and also happens to be a system directory
- **THEN** the out-of-scope rejection is returned — the containment judgement keeps precedence

### Requirement: Workspace root resolving onto a system directory warns at startup

When the resolved workspace root is itself a built-in system directory, the assembling process SHALL log a startup warning naming the resolved root and SHALL start normally — the registration denylist is the safety net, so a too-wide root degrades to denylist-enforced operation rather than misbehaving silently.

#### Scenario: root at a system directory starts with a warning

- **WHEN** the workspace root resolves to `/` (or another built-in system directory) and the WebUI assembles
- **THEN** a warning naming the resolved root is logged at startup, and the server starts and serves normally
