# agent-bench Specification

## Purpose

An evaluation surface for sebas-agent: a headless CLI that runs a fixed task set against a chosen model/client, records the full event stream as JSONL, and reports per-task result scores, pass/fail assertions, and an at-a-glance tree dashboard. No scoring beyond what is defined here — the agent is benchmarked, not graded.

## Requirements

### Requirement: Benchmark CLI and task set

The system SHALL provide a `sebas agent-bench` command that runs a fixed set of tasks (each a prompt + optional project fixture in a temp workspace) against a model/client configuration, records the session's full event stream to a JSONL trace file, and prints a result summary. The benchmark SHALL support `--smoke` (a small fixed subset), result scoring per task, and budget caps per task (model-call and wall-clock limits inherited from the session configuration).

#### Scenario: Smoke run completes with a summary

- **WHEN** `sebas agent-bench --smoke` runs against a fake or real client
- **THEN** the smoke task set finishes within the per-task budget and a summary line lists each task's result, score, and the trace file path

#### Scenario: Static-processing task records a trace

- **WHEN** a task's fixture is processed and the session ends
- **THEN** the JSONL trace contains the task prompt, every tool finish with its result, and the terminal event, in order

### Requirement: Result assertions

The system SHALL determine pass/fail by running deterministic assertions against the recorded trace and the final workspace state — not by inspecting the model's prose. A task SHALL fail-fast when an assertion runs on an unavailable file (missing trace, missing expected output) rather than reporting a false pass.

#### Scenario: Assertions check files, not prose

- **WHEN** a task's expected output is a workspace file with known content
- **THEN** the task passes only if the exact expected content is present after the session, and fails otherwise

#### Scenario: Missing fixture fails fast

- **WHEN** a task's expected output file is absent
- **THEN** the assertion reports failure with the missing path, and no later assertion on that file runs

### Requirement: Error-recovery scenario

The benchmark SHALL include at least one task whose fixture commands fail early and whose score depends on the agent recovering and completing the task despite the failure.

#### Scenario: Recovery task scores recovery

- **WHEN** the fixture issues a failing command and the agent subsequently produces the expected output
- **THEN** the task passes, and the recorded trace shows the failure followed by the successful completion

### Requirement: Tree dashboard

The system SHALL print, at the end of a run, a tree view of the results — tasks grouped by bucket (web-tooling, patching-large-files, sub-agent) with each task's pass/fail, score, and budget flags — using the same layout as the printed assertion report, with no separate reporting UI. The dashboard SHALL be deterministic in ordering (fixed task order, alphabetical within a bucket).

#### Scenario: Tree view groups tasks by bucket

- **WHEN** a benchmark run finishes
- **THEN** the printed tree lists bucket headers with their tasks and a per-bucket pass/fail roll-up, deterministic in ordering

### Requirement: Replay and debug surface

The system SHALL support replaying a recorded trace against a fake client to reproduce the exact event sequence, and SHALL support `--record`-style debugging flags (a `--debug` mode) that print each tool call, its arguments, and its result as the agent runs, so a failing task can be inspected without re-running the whole suite.

#### Scenario: Trace replay reproduces the sequence

- **WHEN** a recorded trace is replayed through the fake client
- **THEN** the replayed event sequence matches the recorded one exactly

### Requirement: Honest environment reporting

The benchmark SHALL record and report the environment the agent ran in — client type, model, tool list, budgets, run time, sample count — in the trace and in the summary, so results are comparable across runs.

#### Scenario: Summary states the environment

- **WHEN** a run prints its summary
- **THEN** the client type, model, tool list, budgets, runtime, and sample count appear in the output and the trace header