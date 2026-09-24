#!/usr/bin/env python3
"""Suite run reports for the three sebas test suites (openspec: add-testsuite-report).

One run → one tree-shaped report, two output forms:

  * an HTML self-contained single file, written to the fixed path
    ``.artifacts/verify/report-<suite>.html`` (overwritten every run), and
  * a compact tree on the terminal (✅/❌/⏭ + indent + per-case seconds).

The data model is deliberately dumb — ``Report → Group (nestable) → Case`` —
so that every acquisition channel (cargo console parsing, nextest JUnit XML,
Playwright JSON) only has to produce the same flat case list with a dotted
group path, and the renderers below are channel-agnostic.

Design references: ``openspec/changes/add-testsuite-report/design.md`` D2/D3/D4.
Standard library only — no new Python dependency (design D2).

Self-test: ``python3 scripts/testsuite_report.py --selftest``.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from typing import Iterable, Optional

#: Fixed report landing directory (spec: 双输出形态与落盘).
ARTIFACT_DIR = os.path.join(".artifacts", "verify")

#: Failure excerpts are truncated into the report (design D4).
FAILURE_EXCERPT_LIMIT = 2000

#: The three suites this change covers. testsuite-real-agents is excluded on
#: purpose (proposal Non-goals: operator-run, no report value).
SUITES = ("e2e", "acceptance", "webui")

PASSED, FAILED, SKIPPED = "passed", "failed", "skipped"

_MARK = {PASSED: "✅", FAILED: "❌", SKIPPED: "⏭"}


# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


@dataclass
class Case:
    """One executed test case.

    ``name`` is the *bare* function/spec title (the thing an operator greps
    for, and the thing the e2e ``--case`` filter matches); ``group_path`` is
    the dotted module/describe chain above it. Title defaults to the name —
    the suites' function names are already the concise titles (spec: 每个 case
    行 SHALL 带简洁标题).
    """

    name: str
    status: str = PASSED
    group_path: tuple = ()
    title: str = ""
    duration_s: Optional[float] = None
    failure_excerpt: str = ""
    sandbox_path: str = ""

    def __post_init__(self):
        self.group_path = tuple(self.group_path)
        if not self.title:
            self.title = self.name

    @property
    def full_name(self) -> str:
        """Dotted name — matches what cargo/nextest/Playwright print."""
        return ".".join(self.group_path + (self.name,))

    @property
    def leaf_status(self) -> str:
        return self.status if self.status in (PASSED, FAILED, SKIPPED) else FAILED


@dataclass
class Group:
    """A node of the report tree (Rust ``mod`` path or Playwright describe)."""

    title: str
    groups: dict = field(default_factory=dict)
    cases: list = field(default_factory=list)

    def child(self, title: str) -> "Group":
        if title not in self.groups:
            self.groups[title] = Group(title)
        return self.groups[title]

    def add(self, case: Case) -> None:
        node = self
        for part in case.group_path:
            node = node.child(part)
        node.cases.append(case)

    def iter_cases(self) -> Iterable[Case]:
        for case in self.cases:
            yield case
        for group in self.groups.values():
            yield from group.iter_cases()

    def counts(self) -> dict:
        """Recursive tallies — every group carries its subtree's counters."""
        tally = {PASSED: 0, FAILED: 0, SKIPPED: 0}
        for case in self.iter_cases():
            tally[case.leaf_status] += 1
        return tally


@dataclass
class Report:
    """One suite run."""

    suite: str
    cases: list = field(default_factory=list)
    suite_duration_s: Optional[float] = None
    channel: str = ""
    note: str = ""
    sandbox_path: str = ""

    def add(self, case: Case) -> None:
        self.cases.append(case)

    def counts(self) -> dict:
        tally = {PASSED: 0, FAILED: 0, SKIPPED: 0}
        for case in self.cases:
            tally[case.leaf_status] += 1
        return tally

    def tree(self) -> Group:
        root = Group(self.suite)
        for case in self.cases:
            root.add(case)
        return root


# ---------------------------------------------------------------------------
# Acquisition channels
# ---------------------------------------------------------------------------


def _group_and_name(dotted: str) -> tuple:
    """Split a dotted test id into (group_path, bare_name).

    ``cargo`` prints either ``suite::case`` or the nested ``suite::mod::case``
    (module reorganization, design D1); nextest and Playwright may use ``::``
    or ``.`` interchangeably. ``support::self`` unit tests living in the
    support module are folded onto the module path as well.
    """
    parts = [p for p in re.split(r"::|\.", dotted.strip()) if p]
    if not parts:
        return (), ""
    # `mod support { … }` unit tests print as `support::self::<case>` — the
    # `self` hop is noise in the tree, drop it.
    parts = [p for p in parts if p != "self"]
    return tuple(parts[:-1]), parts[-1]


_JUNIT_STATUS = {
    "passed": PASSED,
    "failure": FAILED,
    "error": FAILED,
    "skipped": SKIPPED,
}


def parse_nextest_junit(text: str) -> list:
    """Parse ``cargo nextest run --message-format junit`` XML into cases.

    nextest emits one ``<testsuite>`` per binary plus one ``<testsuite>``
    holding the ``<properties>``; each ``<testcase>`` carries ``name`` (dotted
    or ``::``-joined), ``time`` (native per-case seconds) and at most one of
    ``<failure>`` / ``<error>`` / ``<skipped>``.
    """
    try:
        root = ET.fromstring(text.strip())
    except ET.ParseError:
        return []
    cases = []
    for suite_el in root.iter("testsuite"):
        for tc in suite_el.findall("testcase"):
            raw = tc.get("name") or ""
            if not raw:
                continue
            group_path, name = _group_and_name(raw)
            status = PASSED
            excerpt = ""
            for tag in ("failure", "error", "skipped"):
                node = tc.find(tag)
                if node is None:
                    continue
                status = _JUNIT_STATUS[tag]
                excerpt = (node.get("message") or "") + "\n" + (node.text or "")
                break
            cases.append(
                Case(
                    name=name,
                    status=status,
                    group_path=group_path,
                    duration_s=_opt_float(tc.get("time")),
                    failure_excerpt=excerpt.strip(),
                )
            )
    return cases


def parse_webui_shards(paths: Iterable[str]) -> tuple:
    """Merge the Playwright reporter shards into ``(cases, duration_s)``.

    The webui suite is six Playwright runs (main / auth / auth-setup /
    deployment / detached / dead-core configs, see design D2) and each run
    writes its own shard via ``tests/reporters/collect-json.ts``. Cases are
    keyed by Playwright's stable test id so a re-run of one config refreshes
    its slice instead of duplicating it; the describe chain arrives as a
    ``›``-joined title.
    """
    merged = {}
    duration = 0.0
    seen_duration = False
    for path in paths:
        try:
            with open(path, "r", encoding="utf-8") as fh:
                data = json.load(fh)
        except (OSError, ValueError):
            continue
        if not isinstance(data, dict):
            continue
        # `duration` is the run's wall clock in SECONDS (the reporter mirrors
        # Playwright's `stats.duration` ms only for per-case entries).
        shard_duration = data.get("duration")
        if isinstance(shard_duration, (int, float)):
            duration += shard_duration
            seen_duration = True
        for test_id, entry in (data.get("cases") or {}).items():
            if not isinstance(entry, dict):
                continue
            merged[test_id] = entry
    cases = [_webui_shard_case(entry) for entry in merged.values()]
    # Stable presentation: group by path, then by name.
    cases.sort(key=lambda c: (c.group_path, c.name))
    total = duration if seen_duration else None
    return cases, (round(total, 3) if total is not None else None)


def _webui_shard_case(entry: dict) -> Case:
    chain = [part.strip() for part in str(entry.get("title") or "").split("›") if part.strip()]
    if not chain:
        chain = [str(entry.get("id") or "unknown")]
    name = chain[-1]
    group_path = tuple(chain[:-1])
    status = entry.get("status") or PASSED
    if status in ("timedOut", "interrupted", "skipped"):
        status = {"timedOut": FAILED, "interrupted": FAILED, "skipped": SKIPPED}[status]
    elif status not in (PASSED, FAILED, SKIPPED):
        status = PASSED
    duration_ms = entry.get("duration")
    return Case(
        name=name,
        status=status,
        group_path=group_path,
        duration_s=round(duration_ms / 1000.0, 3) if isinstance(duration_ms, (int, float)) else None,
        failure_excerpt=str(entry.get("error") or ""),
    )


def _opt_float(value: Optional[str]) -> Optional[float]:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


#: libtest result line. Anchored on the `test` keyword so unrelated console
#: noise can't be mistaken for a case (design D2: 宽容解析，只认锚行).
_CARGO_TEST_LINE = re.compile(
    r"^test\s+(?P<name>[\w:]+)\s+\.\.\.\s+(?P<status>ok|FAILED|ignored|bench)\b"
)
#: `test result: FAILED. 3 passed; 1 failed; 0 ignored; ...; finished in 1.23s`
_CARGO_RESULT_LINE = re.compile(
    r"^test result:\s+\w+\.\s+(?P<passed>\d+) passed;\s+(?P<failed>\d+) failed;"
    r"\s+(?P<ignored>\d+) ignored"
)
_CARGO_FINISHED = re.compile(r"finished in (?P<secs>[\d.]+)s")

_CARGO_STATUS = {"ok": PASSED, "FAILED": FAILED, "ignored": SKIPPED, "bench": SKIPPED}


def parse_cargo_output(text: str) -> tuple:
    """Parse libtest console output into ``(cases, suite_duration_s)``.

    The cargo fallback channel: per-case *duration* is genuinely unavailable
    (stable libtest without nightly `--report-time`), so ``duration_s`` stays
    ``None`` and only the run total is reported — honest omission, never a
    fabricated number (spec: 降级形态诚实缺省耗时).

    Failure detail is attributed by name from the ``failures:`` block; libtest
    guarantees per-case isolation there even with parallel runs.
    """
    cases = []
    by_name = {}
    failures = _parse_cargo_failures(text)
    suite_duration = None
    for line in text.splitlines():
        stripped = line.strip()
        m = _CARGO_TEST_LINE.match(stripped)
        if m:
            group_path, name = _group_and_name(m.group("name"))
            case = Case(
                name=name,
                status=_CARGO_STATUS.get(m.group("status"), PASSED),
                group_path=group_path,
            )
            cases.append(case)
            by_name[case.full_name] = case
            by_name[name] = case
            continue
        m = _CARGO_RESULT_LINE.match(stripped)
        if m:
            fm = _CARGO_FINISHED.search(stripped)
            if fm:
                suite_duration = _opt_float(fm.group("secs"))
    for full, excerpt in failures.items():
        case = by_name.get(full) or by_name.get(full.rsplit("::", 1)[-1])
        if case is not None and case.status == FAILED:
            case.failure_excerpt = excerpt
    # A FAILED line can appear without a matching failures block (e.g. a
    # process abort); the status still stands.
    if suite_duration is not None:
        suite_duration = round(suite_duration, 3)
    return cases, suite_duration


def _parse_cargo_failures(text: str) -> dict:
    """Extract the ``failures:`` block(s) → ``{test_name: detail_text}``.

    Layout being parsed::

        failures:

        ---- <name> stdout ----
        <body>
        ---- <name> stdout ----

        failures:
            <name>

    Only the labelled sections are harvested; the trailing re-listing carries
    no body and is ignored.
    """
    sections = {}
    current = None
    buffer = []
    for line in text.splitlines():
        m = re.match(r"^-{2,}\s+(\S+)\s+stdout\s+-{2,}\s*$", line.strip())
        if m:
            if current:
                sections[current] = "\n".join(buffer).strip()
            current = m.group(1)
            buffer = []
            continue
        if current is not None:
            if line.strip() == "failures:":
                sections[current] = "\n".join(buffer).strip()
                current = None
                buffer = []
            else:
                buffer.append(line)
    if current:
        sections[current] = "\n".join(buffer).strip()
    return sections


def parse_playwright_json(data) -> tuple:
    """Parse Playwright's JSON reporter output into ``(cases, duration_s)``.

    Accepts the parsed object or a JSON string. The describe chain supplies the
    group path; ``spec.ok``/``results[].status`` decide the status; ``duration``
    is native per-case milliseconds → seconds.
    """
    if isinstance(data, (str, bytes)):
        try:
            data = json.loads(data)
        except ValueError:
            return [], None
    if not isinstance(data, dict):
        return [], None
    cases = []
    for suite in data.get("suites") or []:
        _walk_playwright_suite(suite, (), cases)
    total_ms = data.get("stats", {}).get("duration") if isinstance(data.get("stats"), dict) else None
    duration = round(total_ms / 1000.0, 3) if isinstance(total_ms, (int, float)) else None
    return cases, duration


def _walk_playwright_suite(suite: dict, prefix: tuple, out: list) -> None:
    title = suite.get("title") or ""
    # The file-level suite title is the spec filename; it is redundant with the
    # describe chain and would put `*.spec.ts` in every path — drop it.
    path = prefix if title.endswith((".spec.ts", ".ts")) else prefix + ((title,) if title else ())
    for spec in suite.get("specs") or []:
        out.append(_playwright_case(spec, path))
    for child in suite.get("suites") or []:
        _walk_playwright_suite(child, path, out)


def _playwright_case(spec: dict, path: tuple) -> Case:
    results = spec.get("results") or []
    last = results[-1] if results else {}
    status = last.get("status") or ("passed" if spec.get("ok") else "failed")
    if status in ("timedOut", "interrupted"):
        status = FAILED
    elif status not in (PASSED, FAILED, SKIPPED):
        status = PASSED if spec.get("ok") else FAILED
    excerpt = last.get("error", {}).get("message", "") if isinstance(last.get("error"), dict) else ""
    duration_ms = spec.get("duration")
    if duration_ms is None:
        duration_ms = sum(r.get("duration") or 0 for r in results)
    return Case(
        name=spec.get("title") or "",
        status=status,
        group_path=path,
        duration_s=round(duration_ms / 1000.0, 3) if duration_ms is not None else None,
        failure_excerpt=excerpt or "",
    )


#: Sandboxes are preserved on failure and their path printed by the harness;
#: harvest it from the run output (design D4: 沿用失败保留现场输出约定).
_SANDBOX_PATTERNS = (
    re.compile(r"sandbox scene kept at:\s*(?P<path>\S+)"),
    re.compile(r"kept sandbox dirs? (?:are printed above \(or )?under\s*(?P<path>\S+)"),
    re.compile(r"(?P<path>\S*sbtestsuite\.\S+)"),
    re.compile(r"(?P<path>\S*target[/\\]tests[/\\]\S+)"),
)


def extract_sandbox_path(text: str) -> str:
    """Best-effort sandbox/scene path from the run output; ``""`` if absent."""
    for pattern in _SANDBOX_PATTERNS:
        found = pattern.findall(text)
        if found:
            return found[-1].strip().rstrip(".,;:)")
    return ""


def build_report(suite: str, cases: list, **kwargs) -> Report:
    return Report(suite=suite, cases=list(cases), **kwargs)


# ---------------------------------------------------------------------------
# Rendering
# ---------------------------------------------------------------------------

_STATUS_CLASS = {PASSED: "ok", FAILED: "bad", SKIPPED: "skip"}


def render_terminal(report: Report) -> str:
    """Compact indented tree: ✅/❌/⏭ + title + seconds, sandbox on failures."""
    lines = []
    counts = report.counts()
    header = f"{_MARK[PASSED]} {report.suite} — {counts[PASSED]} passed"
    if counts[FAILED]:
        header += f", {counts[FAILED]} failed"
    if counts[SKIPPED]:
        header += f", {counts[SKIPPED]} skipped"
    if report.suite_duration_s is not None:
        header += f" · {report.suite_duration_s:.1f}s"
    if report.channel:
        header += f" [{report.channel}]"
    lines.append(header)
    if report.note:
        lines.append(f"   ({report.note})")
    lines.append("")
    if not report.cases:
        lines.append("   (0 cases parsed)")
    else:
        _render_terminal_group(report.tree(), lines, 1)
    failures = [c for c in report.cases if c.leaf_status == FAILED]
    if failures:
        lines.append("")
        lines.append("failures:")
        for case in failures:
            lines.append(f"  {_MARK[FAILED]} {case.full_name}")
            if case.sandbox_path:
                lines.append(f"      sandbox: {case.sandbox_path}")
            for excerpt_line in _first_lines(case.failure_excerpt):
                lines.append(f"      {excerpt_line}")
    return "\n".join(lines)


def _render_terminal_group(group: Group, lines: list, depth: int) -> None:
    pad = "   " * depth
    for title, child in group.groups.items():
        counts = child.counts()
        lines.append(f"{pad}{title}/ ({_counts_brief(counts)})")
        _render_terminal_group(child, lines, depth + 1)
    for case in group.cases:
        lines.append(f"{pad}{_MARK[case.leaf_status]} {case.title}{_secs(case)}")
        if case.leaf_status == FAILED and case.sandbox_path:
            lines.append(f"{pad}      sandbox: {case.sandbox_path}")


def _first_lines(excerpt: str, limit: int = 3, width: int = 150) -> list:
    out = []
    for line in (excerpt or "").splitlines():
        line = line.strip()
        if line:
            out.append(line[:width])
        if len(out) >= limit:
            break
    return out


def _counts_brief(counts: dict) -> str:
    bits = [f"{counts[PASSED]}✅"]
    if counts[FAILED]:
        bits.append(f"{counts[FAILED]}❌")
    if counts[SKIPPED]:
        bits.append(f"{counts[SKIPPED]}⏭")
    return " ".join(bits)


def _secs(case: Case) -> str:
    return f" ({case.duration_s:.1f}s)" if case.duration_s is not None else ""


def truncate_excerpt(text: str, limit: int = FAILURE_EXCERPT_LIMIT) -> str:
    """Clamp a failure body for embedding (design D4, ~2000 chars)."""
    text = (text or "").strip()
    if len(text) <= limit:
        return text
    return text[:limit] + f"\n… [truncated, {len(text) - limit} more chars]"


def render_html(report: Report) -> str:
    """Self-contained single file: inline CSS, zero external resources."""
    counts = report.counts()
    parts = [
        "<!DOCTYPE html>",
        '<html lang="zh"><head><meta charset="utf-8">',
        f"<title>testsuite-{html.escape(report.suite)} report</title>",
        f"<style>{_CSS}</style>",
        "</head><body>",
        "<header>",
        f"<h1>testsuite-{html.escape(report.suite)}</h1>",
        '<p class="meta">'
        f'<span class="pill ok">{counts[PASSED]} passed</span>'
        f'<span class="pill bad">{counts[FAILED]} failed</span>'
        f'<span class="pill skip">{counts[SKIPPED]} skipped</span>'
        + (
            f'<span class="pill">{report.suite_duration_s:.1f}s total</span>'
            if report.suite_duration_s is not None
            else ""
        )
        + (f'<span class="pill">channel: {html.escape(report.channel)}</span>' if report.channel else "")
        + "</p>",
    ]
    if report.note:
        parts.append(f'<p class="note">{html.escape(report.note)}</p>')
    if report.sandbox_path:
        parts.append(f'<p class="note">sandbox: <code>{html.escape(report.sandbox_path)}</code></p>')
    parts.append("</header>")
    if not report.cases:
        parts.append('<p class="note">0 cases parsed — the acquisition channel produced no case lines.</p>')
    else:
        parts.append("<ul class=\"tree\">")
        _render_html_group(report.tree(), parts, 1)
        parts.append("</ul>")
    parts.append("</body></html>")
    return "\n".join(parts)


def _render_html_group(group: Group, parts: list, depth: int) -> None:
    for title, child in group.groups.items():
        counts = child.counts()
        cls = "bad" if counts[FAILED] else "group"
        parts.append(
            f'<li class="group {cls}"><span class="gt">{html.escape(title)}/</span>'
            f'<span class="gc">{_counts_brief(counts)}</span>'
        )
        parts.append("<ul>")
        _render_html_group(child, parts, depth + 1)
        parts.append("</ul></li>")
    for case in group.cases:
        cls = _STATUS_CLASS[case.leaf_status]
        parts.append(
            f'<li class="case {cls}"><span class="mark">{_MARK[case.leaf_status]}</span>'
            f'<span class="name" title="{html.escape(case.full_name)}">{html.escape(case.title)}</span>'
            f'<span class="dur">{_secs(case).strip() or "—"}</span>'
        )
        if case.leaf_status == FAILED and (case.failure_excerpt or case.sandbox_path):
            parts.append('<div class="detail">')
            if case.sandbox_path:
                parts.append(f'<div class="sandbox">sandbox: <code>{html.escape(case.sandbox_path)}</code></div>')
            if case.failure_excerpt:
                parts.append(
                    f"<pre>{html.escape(truncate_excerpt(case.failure_excerpt))}</pre>"
                )
            parts.append("</div>")
        parts.append("</li>")


_CSS = """
:root { color-scheme: light dark; }
body { font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
       margin: 0; padding: 24px 32px; background: #fbfbfd; color: #1c1c22; }
h1 { font-size: 20px; margin: 0 0 8px; }
header { border-bottom: 1px solid #d8d8e0; padding-bottom: 12px; margin-bottom: 16px; }
.meta { margin: 0; display: flex; gap: 8px; flex-wrap: wrap; }
.pill { border-radius: 10px; padding: 1px 9px; background: #ececf2; font-size: 12px; }
.pill.ok { background: #d9f2df; } .pill.bad { background: #fbdada; } .pill.skip { background: #eee; }
.note { color: #6a6a78; font-size: 12px; margin: 6px 0 0; }
ul.tree { list-style: none; margin: 0; padding-left: 0; }
ul.tree ul { list-style: none; margin: 0; padding-left: 20px; border-left: 1px solid #e2e2ea; }
li.group { margin-top: 10px; }
.gt { font-weight: 600; }
.gc { color: #6a6a78; font-size: 12px; margin-left: 8px; }
li.case { padding: 1px 0; }
li.case .mark { margin-right: 6px; }
li.case.bad > .name { color: #a3141b; font-weight: 600; }
li.case.skip > .name { color: #6a6a78; }
.dur { color: #8a8a97; font-size: 12px; margin-left: 8px; }
.detail { margin: 6px 0 10px 24px; }
.sandbox { color: #6a6a78; font-size: 12px; }
pre { background: #f4f4f8; border: 1px solid #e2e2ea; border-radius: 6px;
      padding: 8px 10px; overflow-x: auto; white-space: pre-wrap; font-size: 12px; }
@media (prefers-color-scheme: dark) {
  body { background: #16161c; color: #e6e6ee; }
  header { border-color: #33333d; }
  .pill { background: #2a2a34; } .pill.ok { background: #1d3a26; } .pill.bad { background: #3d1e20; }
  ul.tree ul { border-color: #2a2a34; }
  pre { background: #1e1e26; border-color: #33333d; }
  li.case.bad > .name { color: #ff9aa0; }
}
"""


# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------


def report_path(suite: str) -> str:
    return os.path.join(ARTIFACT_DIR, f"report-{suite}.html")


def write_report(report: Report, path: Optional[str] = None) -> str:
    """Write the HTML report; parent dirs are created on demand.

    Raises ``OSError`` on failure — callers treat that as a warning, never as a
    suite failure (spec: 报告产出 MUST NOT 改变套件退出码语义).
    """
    target = path or report_path(report.suite)
    parent = os.path.dirname(target)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(target, "w", encoding="utf-8") as fh:
        fh.write(render_html(report))
    return target


def emit(report: Report, path: Optional[str] = None, warn=None) -> Optional[str]:
    """Print the terminal tree and write the HTML; never raises.

    Returns the written path, or ``None`` when the report could not be written
    — the warning goes through ``warn`` (defaults to stderr) and the caller's
    exit code is untouched.
    """
    print(render_terminal(report), flush=True)
    try:
        target = write_report(report, path)
    except OSError as exc:
        (warn or _default_warn)(f"[report] WARNING: could not write report: {exc}")
        return None
    print(f"[report] {target}", flush=True)
    return target


def _default_warn(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


# ---------------------------------------------------------------------------
# CLI — invoked by tasks.py after a suite has run
# ---------------------------------------------------------------------------


def read_text(path: str) -> str:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError:
        return ""


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Render a sebas testsuite run report.")
    # --selftest runs the embedded unit tests and needs no suite.
    parser.add_argument("--suite", choices=SUITES, help="suite name (required unless --selftest)")
    parser.add_argument(
        "--channel",
        default="auto",
        choices=("auto", "junit", "cargo", "playwright", "webui-shards"),
        help="acquisition channel; 'auto' sniffs the input",
    )
    parser.add_argument("--input", help="captured run output (file)")
    parser.add_argument(
        "--shards",
        nargs="*",
        default=[],
        help="webui reporter shard files (webui-shards channel)",
    )
    parser.add_argument("--out", help="HTML target; defaults to the fixed artifact path")
    parser.add_argument("--duration", type=float, help="suite wall-clock seconds (harness timer)")
    parser.add_argument("--note", default="", help="free-form note rendered in the report")
    parser.add_argument("--selftest", action="store_true", help="run the embedded unit tests")
    args = parser.parse_args(argv)

    if args.selftest:
        return selftest()
    if not args.suite:
        parser.error("--suite is required")

    text = read_text(args.input) if args.input else ""
    channel = args.channel
    if channel == "auto":
        channel = "junit" if text.lstrip().startswith("<") else ("playwright" if args.suite == "webui" else "cargo")

    if channel == "junit":
        cases, suite_duration = parse_nextest_junit(text), None
    elif channel == "webui-shards":
        cases, suite_duration = parse_webui_shards(args.shards or [args.input] if args.input else args.shards)
    elif channel == "playwright":
        cases, suite_duration = parse_playwright_json(text)
    else:
        cases, suite_duration = parse_cargo_output(text)

    if args.duration is not None:
        suite_duration = round(args.duration, 3)
    if channel == "cargo" and cases:
        note = args.note or "cargo fallback channel: no per-case duration available"
    else:
        note = args.note

    report = build_report(
        args.suite,
        cases,
        suite_duration_s=suite_duration,
        channel=channel,
        note=note,
        sandbox_path=extract_sandbox_path(text),
    )
    emit(report, args.out)
    return 0


# ---------------------------------------------------------------------------
# Self-test (task 1.1: 单测覆盖渲染与截断)
# ---------------------------------------------------------------------------


def selftest() -> int:
    import unittest

    class ReportTests(unittest.TestCase):
        def test_tree_groups_by_module_path(self):
            report = build_report(
                "e2e",
                [
                    Case("alpha", group_path=("channel_and_supervision",)),
                    Case("beta", group_path=("channel_and_supervision", "secrets")),
                    Case("gamma"),
                ],
            )
            root = report.tree()
            self.assertIn("channel_and_supervision", root.groups)
            self.assertEqual(
                [c.name for c in root.groups["channel_and_supervision"].cases], ["alpha"]
            )
            self.assertEqual(
                [c.name for c in root.groups["channel_and_supervision"].groups["secrets"].cases],
                ["beta"],
            )
            self.assertEqual([c.name for c in root.cases], ["gamma"])

        def test_group_counts_are_recursive(self):
            report = build_report(
                "e2e",
                [
                    Case("a", group_path=("g",), status=PASSED),
                    Case("b", group_path=("g", "h"), status=FAILED),
                    Case("c", group_path=("g", "h"), status=SKIPPED),
                ],
            )
            counts = report.tree().groups["g"].counts()
            self.assertEqual(counts, {PASSED: 1, FAILED: 1, SKIPPED: 1})

        def test_full_name_is_dotted(self):
            case = Case("switch", group_path=("mode", "session"))
            self.assertEqual(case.full_name, "mode.session.switch")

        def test_terminal_tree_marks_and_indent(self):
            report = build_report(
                "e2e",
                [
                    Case("a", group_path=("grp",), duration_s=1.25),
                    Case("b", group_path=("grp",), status=FAILED, sandbox_path="/tmp/sb1"),
                ],
                suite_duration_s=12.0,
                channel="cargo",
            )
            out = render_terminal(report)
            self.assertIn("grp/", out)
            self.assertIn("✅ a (1.2s)", out)
            self.assertIn("❌ b", out)
            self.assertIn("sandbox: /tmp/sb1", out)
            self.assertIn("[cargo]", out)

        def test_terminal_marks_skipped(self):
            out = render_terminal(
                build_report("e2e", [Case("s", status=SKIPPED, duration_s=0.0)])
            )
            self.assertIn("⏭ s (0.0s)", out)

        def test_terminal_omits_duration_when_unknown(self):
            out = render_terminal(build_report("e2e", [Case("nodur")]))
            self.assertIn("✅ nodur", out)
            self.assertNotIn("nodur (", out)

        def test_terminal_reports_zero_cases_honestly(self):
            self.assertIn("(0 cases parsed)", render_terminal(build_report("e2e", [])))

        def test_html_is_self_contained(self):
            html_text = render_html(build_report("webui", [Case("smoke", group_path=("login",))]))
            self.assertTrue(html_text.startswith("<!DOCTYPE html>"))
            self.assertIn("login/", html_text)
            self.assertIn("✅", html_text)
            # No external resource can be fetched by the browser.
            self.assertNotIn("<script", html_text)
            self.assertNotIn("http://", html_text)
            self.assertNotIn("https://", html_text)

        def test_html_escapes_markup(self):
            html_text = render_html(
                build_report("e2e", [Case("<img src=x onerror=alert(1)>")])
            )
            self.assertNotIn("<img src=x", html_text)
            self.assertIn("&lt;img", html_text)

        def test_html_failure_detail_and_sandbox(self):
            html_text = render_html(
                build_report(
                    "e2e",
                    [Case("boom", status=FAILED, failure_excerpt="assertion failed", sandbox_path="/tmp/sb2")],
                )
            )
            self.assertIn("assertion failed", html_text)
            self.assertIn("/tmp/sb2", html_text)

        def test_truncate_excerpt_clamps(self):
            long_text = "x" * (FAILURE_EXCERPT_LIMIT + 500)
            out = truncate_excerpt(long_text)
            self.assertTrue(out.startswith("x" * 100))
            self.assertIn("truncated, 500 more chars", out)
            self.assertLess(len(out), len(long_text) + 60)

        def test_truncate_excerpt_passes_short_text(self):
            self.assertEqual(truncate_excerpt("  short  "), "short")
            self.assertEqual(truncate_excerpt(""), "")

    class CargoParserTests(unittest.TestCase):
        SAMPLE = (
            "running 3 tests\n"
            "test channel_and_supervision::secret_rotation_self_heal ... ok\n"
            "test session_lifecycle::session_round_trip ... FAILED\n"
            "test misc::skipped_thing ... ignored\n"
            "\nfailures:\n\n"
            "---- session_lifecycle::session_round_trip stdout ----\n"
            "thread 'main' panicked at 'boom'\n"
            "assertion `left == right` failed\n"
            "\n"
            "failures:\n"
            "    session_lifecycle::session_round_trip\n"
            "\n"
            "test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 4.25s\n"
        )

        def test_parses_statuses_and_group_paths(self):
            cases, duration = parse_cargo_output(self.SAMPLE)
            by_name = {c.name: c for c in cases}
            self.assertEqual(set(by_name), {"secret_rotation_self_heal", "session_round_trip", "skipped_thing"})
            self.assertEqual(by_name["secret_rotation_self_heal"].status, PASSED)
            self.assertEqual(by_name["session_round_trip"].status, FAILED)
            self.assertEqual(by_name["skipped_thing"].status, SKIPPED)
            self.assertEqual(by_name["session_round_trip"].group_path, ("session_lifecycle",))
            self.assertEqual(duration, 4.25)

        def test_never_fabricates_per_case_duration(self):
            cases, _ = parse_cargo_output(self.SAMPLE)
            self.assertTrue(all(c.duration_s is None for c in cases))

        def test_attributes_failure_excerpt_by_name(self):
            cases, _ = parse_cargo_output(self.SAMPLE)
            failing = next(c for c in cases if c.name == "session_round_trip")
            self.assertIn("panicked at 'boom'", failing.failure_excerpt)
            passing = next(c for c in cases if c.name == "secret_rotation_self_heal")
            self.assertEqual(passing.failure_excerpt, "")

        def test_unknown_lines_are_ignored(self):
            cases, duration = parse_cargo_output("Compiling sebas v0.1.0\nsome other noise\n")
            self.assertEqual(cases, [])
            self.assertIsNone(duration)

        def test_self_hop_is_dropped(self):
            cases, _ = parse_cargo_output("test support::self::helper_case ... ok\n")
            self.assertEqual(cases[0].group_path, ("support",))

    class JunitParserTests(unittest.TestCase):
        XML = """<?xml version="1.0" encoding="UTF-8"?>
<testsuites>
  <testsuite name="testsuite_e2e_test" tests="3">
    <testcase name="channel_and_supervision::secret_rotation" time="1.5"/>
    <testcase name="mode::mode_mid_session_switch" time="2.25">
      <failure message="assertion failed">thread panicked\nstack backtrace</failure>
    </testcase>
    <testcase name="misc::skipped_one" time="0.0">
      <skipped message="no claude bin"/>
    </testcase>
  </testsuite>
</testsuites>
"""

        def test_parses_cases(self):
            cases = parse_nextest_junit(self.XML)
            self.assertEqual([c.name for c in cases], ["secret_rotation", "mode_mid_session_switch", "skipped_one"])

        def test_native_durations(self):
            cases = parse_nextest_junit(self.XML)
            self.assertEqual(cases[0].duration_s, 1.5)
            self.assertEqual(cases[1].duration_s, 2.25)

        def test_statuses_and_excerpt(self):
            cases = parse_nextest_junit(self.XML)
            self.assertEqual(cases[0].status, PASSED)
            self.assertEqual(cases[1].status, FAILED)
            self.assertEqual(cases[2].status, SKIPPED)
            self.assertIn("stack backtrace", cases[1].failure_excerpt)

        def test_group_paths(self):
            cases = parse_nextest_junit(self.XML)
            self.assertEqual(cases[1].group_path, ("mode",))

        def test_malformed_xml_is_tolerated(self):
            self.assertEqual(parse_nextest_junit("<testsuites><oops"), [])

    class PlaywrightParserTests(unittest.TestCase):
        DATA = {
            "stats": {"duration": 12345},
            "suites": [
                {
                    "title": "first-paint.spec.ts",
                    "specs": [],
                    "suites": [
                        {
                            "title": "首屏",
                            "specs": [],
                            "suites": [
                                {
                                    "title": "渲染",
                                    "specs": [
                                        {"title": "paints the shell", "ok": True, "duration": 250, "results": [{"status": "passed", "duration": 250}]},
                                        {
                                            "title": "keeps the rail",
                                            "ok": False,
                                            "duration": 900,
                                            "results": [
                                                {"status": "timedOut", "duration": 900, "error": {"message": "Timeout 30000ms exceeded"}}
                                            ],
                                        },
                                    ],
                                }
                            ],
                        }
                    ],
                }
            ],
        }

        def test_describe_chain_becomes_group_path(self):
            cases, _ = parse_playwright_json(self.DATA)
            self.assertEqual(cases[0].group_path, ("首屏", "渲染"))
            self.assertNotIn("first-paint.spec.ts", ".".join(cases[0].group_path))

        def test_spec_file_title_is_dropped(self):
            cases, _ = parse_playwright_json(self.DATA)
            self.assertTrue(all(not part.endswith(".spec.ts") for c in cases for part in c.group_path))

        def test_statuses_and_durations(self):
            cases, duration = parse_playwright_json(self.DATA)
            self.assertEqual(cases[0].status, PASSED)
            self.assertEqual(cases[0].duration_s, 0.25)
            self.assertEqual(cases[1].status, FAILED)
            self.assertEqual(cases[1].duration_s, 0.9)
            self.assertEqual(duration, 12.345)

        def test_error_message_becomes_excerpt(self):
            cases, _ = parse_playwright_json(self.DATA)
            self.assertIn("Timeout 30000ms exceeded", cases[1].failure_excerpt)

        def test_accepts_raw_json_text(self):
            cases, _ = parse_playwright_json(json.dumps(self.DATA))
            self.assertEqual(len(cases), 2)

        def test_garbage_is_tolerated(self):
            self.assertEqual(parse_playwright_json("not json"), ([], None))

    class SandboxPathTests(unittest.TestCase):
        def test_scene_kept_line(self):
            self.assertEqual(
                extract_sandbox_path("[testsuite] tests failed — sandbox scene kept at: /tmp/sbtestsuite.abc"),
                "/tmp/sbtestsuite.abc",
            )

        def test_rust_kept_dir_hint(self):
            self.assertEqual(
                extract_sandbox_path("kept sandbox dirs are printed above (or under target/tests/)"),
                "target/tests/",
            )

        def test_absent_returns_empty(self):
            self.assertEqual(extract_sandbox_path("all green"), "")

    class WebuiShardTests(unittest.TestCase):
        def _write(self, tmp, name, cases, duration=None):
            import json as _json

            path = os.path.join(tmp, name)
            with open(path, "w", encoding="utf-8") as fh:
                _json.dump({"cases": cases, "duration": duration}, fh)
            return path

        def test_merges_multiple_config_shards_into_one_tree(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                main = self._write(
                    tmp,
                    "webui-results.json",
                    {"a": {"id": "a", "title": "首屏 › 渲染 › paints", "status": "passed", "duration": 100}},
                    1.0,
                )
                auth = self._write(
                    tmp,
                    "webui-results-auth.json",
                    {"b": {"id": "b", "title": "登录 › signs in", "status": "passed", "duration": 200}},
                    2.0,
                )
                cases, duration = parse_webui_shards([main, auth])
            by_name = {c.name: c for c in cases}
            self.assertEqual(set(by_name), {"paints", "signs in"})
            self.assertEqual(by_name["paints"].group_path, ("首屏", "渲染"))
            self.assertEqual(duration, 3.0)

        def test_same_test_id_across_shards_is_refreshed_not_duplicated(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                first = self._write(
                    tmp, "a.json", {"x": {"id": "x", "title": "g › t", "status": "failed", "duration": 10}}
                )
                second = self._write(
                    tmp, "b.json", {"x": {"id": "x", "title": "g › t", "status": "passed", "duration": 20}}
                )
                cases, _ = parse_webui_shards([first, second])
            self.assertEqual(len(cases), 1)
            self.assertEqual(cases[0].status, PASSED)
            self.assertEqual(cases[0].duration_s, 0.02)

        def test_timeout_and_skipped_statuses(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                path = self._write(
                    tmp,
                    "a.json",
                    {
                        "t": {"id": "t", "title": "g › times out", "status": "timedOut", "duration": 30000},
                        "s": {"id": "s", "title": "g › skips", "status": "skipped", "duration": 0},
                    },
                )
                cases, _ = parse_webui_shards([path])
            by_name = {c.name: c for c in cases}
            self.assertEqual(by_name["times out"].status, FAILED)
            self.assertEqual(by_name["skips"].status, SKIPPED)

        def test_error_text_becomes_excerpt(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                path = self._write(
                    tmp,
                    "a.json",
                    {"t": {"id": "t", "title": "g › broken", "status": "failed", "duration": 5, "error": "boom"}},
                )
                cases, _ = parse_webui_shards([path])
            self.assertEqual(cases[0].failure_excerpt, "boom")

        def test_missing_shard_is_tolerated(self):
            cases, duration = parse_webui_shards(["/nonexistent/shard.json"])
            self.assertEqual(cases, [])
            self.assertIsNone(duration)

        def test_case_without_title_falls_back_to_id(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                path = self._write(tmp, "a.json", {"the-id": {"id": "the-id", "status": "passed"}})
                cases, _ = parse_webui_shards([path])
            self.assertEqual(cases[0].name, "the-id")

    class OutputTests(unittest.TestCase):
        def test_report_path_is_fixed(self):
            self.assertEqual(report_path("e2e"), os.path.join(".artifacts", "verify", "report-e2e.html"))

        def test_write_report_creates_parents(self):
            import tempfile

            with tempfile.TemporaryDirectory() as tmp:
                target = os.path.join(tmp, "nested", "report-e2e.html")
                written = write_report(build_report("e2e", [Case("a")]), target)
                self.assertEqual(written, target)
                self.assertIn('<span class="name" title="a">a</span>', read_text(target))

        def test_emit_warns_instead_of_raising(self):
            import tempfile

            warnings = []
            with tempfile.TemporaryDirectory() as tmp:
                blocker = os.path.join(tmp, "blocker")
                with open(blocker, "w") as fh:
                    fh.write("")
                # `blocker` is a file, so it cannot serve as a parent dir.
                result = emit(
                    build_report("e2e", [Case("a")]),
                    os.path.join(blocker, "report-e2e.html"),
                    warn=warnings.append,
                )
            self.assertIsNone(result)
            self.assertEqual(len(warnings), 1)
            self.assertIn("WARNING", warnings[0])

    runner = unittest.TextTestRunner(verbosity=2)
    suite = unittest.TestSuite()
    loader = unittest.TestLoader()
    for klass in (
        ReportTests,
        CargoParserTests,
        JunitParserTests,
        PlaywrightParserTests,
        WebuiShardTests,
        SandboxPathTests,
        OutputTests,
    ):
        suite.addTests(loader.loadTestsFromTestCase(klass))
    return 0 if runner.run(suite).wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())