//! `check-docs` — ghost-reference guard for design docs.
//!
//! sebas removed its pre-OpenSpec planning corpus (`docs/superpowers/`)
//! in 2026-08. This check keeps the source tree free of citations that
//! point at it:
//!
//! - literal `docs/superpowers/` paths
//! - dated citations: `spec 2026-08-17 §2.5` (pattern `spec <YYYY-MM-DD>`)
//! - bare section citations: `spec §4.2` (pattern `spec §<digit>`)
//!
//! Live documentation should cite `openspec/specs/<capability>/spec.md`
//! or `docs/design-history.md` instead. The scanner skips generated and
//! planning trees where those patterns legitimately appear as prose.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Directories never scanned (relative to repo root). `openspec/` is
/// excluded wholesale: capability specs use `### Requirement:` headings,
/// not § citations, and change-planning documents legitimately quote the
/// banned patterns while describing this policy.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".beads",
    "target",
    "node_modules",
    "openspec",
    "state",
];

/// Files never scanned. `docs/design-history.md` deliberately records the
/// original (deleted) document paths for git archaeology; this scanner's
/// own source contains the banned patterns as test fixtures.
const SKIP_FILES: &[&str] = &["design-history.md", "check_docs.rs"];

const SCANNED_EXTENSIONS: &[&str] = &["rs", "toml", "md"];

/// One citation hit: file path, 1-based line number, and the trimmed line.
#[derive(Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub line_no: usize,
    pub line: String,
}

/// Run the scan rooted at `root`. Returns every offending line, sorted by
/// path then line number.
pub fn scan(root: &Path) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if path.is_dir() {
                if SKIP_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                stack.push(path);
            } else if is_scannable(&path, &name) {
                scan_file(&path, &mut hits);
            }
        }
    }
    hits.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_no.cmp(&b.line_no)));
    hits
}

fn is_scannable(path: &Path, name: &str) -> bool {
    if SKIP_FILES.contains(&name) {
        return false;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| SCANNED_EXTENSIONS.contains(&ext))
}

fn scan_file(path: &Path, hits: &mut Vec<Hit>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // non-UTF8 (e.g. lockfile binaries) — skip
    };
    for (idx, line) in content.lines().enumerate() {
        if line_has_citation(line) {
            hits.push(Hit {
                path: path.to_path_buf(),
                line_no: idx + 1,
                line: line.trim().to_string(),
            });
        }
    }
}

/// True when the line cites the removed corpus: a `docs/superpowers/`
/// path, a dated `spec YYYY-MM-DD` citation (any case), a bare `spec §N`
/// one, or a fabricated `spec.md §N` section reference (OpenSpec specs
/// use `### Requirement:` headings, never § numbering).
pub fn line_has_citation(line: &str) -> bool {
    if line.contains("docs/superpowers/") {
        return true;
    }
    let lower = line.to_lowercase();
    // `spec 2026-08-17` — "spec " followed by 4 digits, then 2x(-2 digits).
    if let Some(pos) = lower.find("spec ") {
        let rest = &lower[pos + 5..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.len() == 4 {
            let tail = &rest[4..];
            if tail.starts_with('-') {
                let tail_digits: Vec<char> =
                    tail.chars().skip(1).take_while(|c| c.is_ascii_digit()).collect();
                if tail_digits.len() == 2 {
                    return true;
                }
            }
        }
    }
    // `spec §4` / `spec.md §4` — "spec[.md] §" followed by a digit.
    // `§` is 2 bytes in UTF-8; find the marker instead of slicing bytes.
    for marker in ["spec §", "spec.md §"] {
        if let Some(pos) = lower.find(marker) {
            let rest = &lower[pos + marker.len()..];
            if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// CLI entry: `xtask check-docs [root]`. Root defaults to the repo root
/// inferred from `CARGO_MANIFEST_DIR` (xtask lives at `<repo>/xtask/`).
pub fn run(extra_args: &[String]) -> ExitCode {
    if extra_args.iter().any(|a| a == "--help" || a == "-h") {
        println!("xtask check-docs — fail if the tree cites the removed docs/superpowers corpus");
        return ExitCode::SUCCESS;
    }
    let root = match extra_args.first() {
        Some(p) => PathBuf::from(p),
        None => match std::env::var("CARGO_MANIFEST_DIR") {
            Ok(dir) => Path::new(&dir)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
            Err(_) => PathBuf::from("."),
        },
    };
    if !root.is_dir() {
        eprintln!("xtask check-docs: root `{}` is not a directory", root.display());
        return ExitCode::from(2);
    }

    let hits = scan(&root);
    if hits.is_empty() {
        println!("check-docs: no ghost references to docs/superpowers found");
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "check-docs: {} ghost reference(s) to the removed docs/superpowers corpus:",
        hits.len()
    );
    for hit in &hits {
        eprintln!("  {}:{}: {}", hit.path.display(), hit.line_no, hit.line);
    }
    eprintln!("fix: point at openspec/specs/<capability>/spec.md or docs/design-history.md");
    ExitCode::FAILURE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xtask-check-docs-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn flags_every_citation_form() {
        assert!(line_has_citation("//! See docs/superpowers/specs/x.md."));
        assert!(line_has_citation("// spec 2026-08-17 §2.5 renamed this"));
        assert!(line_has_citation("// Spec 2026-08-17 §2.2: in-band signal"));
        assert!(line_has_citation("/// 幂等（spec §4.2）。"));
        assert!(line_has_citation("/// see openspec/specs/watchdog/spec.md §7")); // fabricated §N
        assert!(!line_has_citation("openspec/specs/feishu-cards/spec.md"));
        assert!(!line_has_citation("docs/design-history.md ADR-1"));
        assert!(!line_has_citation("special 2026-08-17 date-ish text")); // "special " ≠ "spec "
        assert!(!line_has_citation("the specs/2026 folder listing"));
        assert!(!line_has_citation("spike §S6 wire frames")); // non-spec citation, out of scope
    }

    #[test]
    fn scan_finds_hits_and_honors_exclusions() {
        let root = temp_root("hits");
        fs::create_dir_all(root.join("target")).unwrap();
        fs::create_dir_all(root.join("openspec/changes/archive")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("clean.rs"), "// fine: openspec/specs/x/spec.md\n").unwrap();
        fs::write(
            root.join("guilty.rs"),
            "//! See docs/superpowers/specs/2026-07-26-sebas-design.md.\n",
        )
        .unwrap();
        // excluded tree + exempt file must not produce hits
        fs::write(
            root.join("target/guilty.rs"),
            "// docs/superpowers/ in target\n",
        )
        .unwrap();
        fs::write(
            root.join("openspec/changes/archive/proposal.md"),
            "# spec §4.2 in archive\n",
        )
        .unwrap();
        fs::write(
            root.join("docs/design-history.md"),
            "原文: docs/superpowers/specs/2026-08-06-router-design.md\n",
        )
        .unwrap();

        let hits = scan(&root);
        assert_eq!(hits.len(), 1, "exactly the guilty source line: {hits:?}");
        assert!(hits[0].path.ends_with("guilty.rs"));
        assert_eq!(hits[0].line_no, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_clean_tree_returns_empty() {
        let root = temp_root("clean");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "//! Behavior: openspec/specs/router-core/spec.md; rationale: docs/design-history.md\n",
        )
        .unwrap();
        assert!(scan(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
