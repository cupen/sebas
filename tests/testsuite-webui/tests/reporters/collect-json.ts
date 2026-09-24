import fs from 'node:fs'
import path from 'node:path'
import type {
  FullResult,
  Reporter,
  Suite,
  TestCase,
  TestResult,
} from '@playwright/test/reporter'

/**
 * Machine-readable result collection for the terminal report (openspec:
 * add-testsuite-report).
 *
 * Sits next to `keep-on-fail.ts` in the same reporter array and is deliberately
 * independent of it: keep-on-fail owns the sandbox scene keep/clean decision,
 * this one only records what ran so `scripts/testsuite_report.py` can render
 * `.artifacts/verify/report-webui.html` + the terminal tree.
 *
 * The webui suite is **six Playwright runs** (main / auth / auth-setup /
 * deployment / detached / dead-core configs), so each run writes its own shard
 * and the report generator merges the shards into one tree (design D2/D3). The
 * shard is keyed by the config file stem — `playwright.config.ts` → the bare
 * `<out>.json`, every other config → `<out>-<stem>.json`. Later shards merge
 * into earlier ones by test id, so re-running one config only refreshes its
 * slice instead of dropping the rest.
 *
 * Where the shard goes:
 *   - `TESTSUITE_REPORT_JSON` (env) — tasks.py pins `.artifacts/verify/` here;
 *   - otherwise `.artifacts/verify/webui-results.json` relative to the repo
 *     root, so a bare `pnpm exec playwright test` still produces something.
 *
 * Reporting must never change the suite's pass/fail verdict: every write is
 * best-effort and swallows its own errors.
 */

// reporters/ → tests/ → testsuite-webui/ → tests/ → repo root: four levels up.
// (Three would land on `<repo>/tests`, which silently misfiled every shard as
// `<repo>/tests/.artifacts/verify/` and made the report render 0 cases.)
const REPO_ROOT = path.resolve(import.meta.dirname, '../../../..')

const OUT_BASE =
  process.env.TESTSUITE_REPORT_JSON ?? path.join(REPO_ROOT, '.artifacts', 'verify', 'webui-results.json')

/** Shards carry the config identity so the six runs stay distinguishable.
 *  `playwright.config.ts` → `<out>.json`; e.g. `playwright.auth.config.ts` →
 *  `<out>-auth.json`. tasks.py merges exactly this set. */
const shardPath = (configFile: string): string => {
  const base = path.basename(configFile || 'playwright.config.ts')
  // Strip the `.config.ts` suffix FIRST, then the `playwright.` prefix, so
  // `playwright.config.ts` → '' (the bare shard) and `playwright.auth.config.ts`
  // → 'auth'.
  const stem = base.replace(/\.config\.ts$/, '').replace(/^playwright\.?/, '')
  if (!stem) return OUT_BASE
  return OUT_BASE.replace(/\.json$/, `-${stem}.json`)
}

const configFileOf = (config: unknown): string =>
  (config as { configFile?: string })?.configFile ?? 'playwright.config.ts'

interface ShardCase {
  /** Dotted describe chain + title — the tree path the generator splits on. */
  title: string
  /** Playwright's own id (`file › describe › title`), used for stable merge. */
  id: string
  file: string
  project: string
  status: string
  duration: number
  error: string
}

const serialize = (test: TestCase, result: TestResult, project: string): ShardCase => ({
  title: test.titlePath().filter(Boolean).join(' › '),
  id: test.id,
  file: path.relative(REPO_ROOT, test.location.file),
  project,
  // `result.status` is the last attempt's status; retries are folded in, which
  // is what an operator wants on the report line.
  status: result.status,
  duration: result.duration,
  error: result.error?.message ?? '',
})

const readShard = (file: string): Record<string, ShardCase> => {
  try {
    const parsed = JSON.parse(fs.readFileSync(file, 'utf8')) as { cases?: Record<string, ShardCase> }
    return parsed?.cases && typeof parsed.cases === 'object' ? parsed.cases : {}
  } catch {
    return {} // missing or corrupt shard → start fresh
  }
}

class CollectJson implements Reporter {
  private readonly cases = new Map<string, ShardCase>()
  private target: string
  private configFile: string

  constructor() {
    this.configFile = configFileOf(undefined)
    this.target = shardPath(this.configFile)
  }

  onBegin(config: unknown): void {
    // The real config file is only known once the run begins.
    const file = shardPath(configFileOf(config))
    this.configFile = configFileOf(config)
    this.target = file
    for (const [id, entry] of Object.entries(readShard(file))) this.cases.set(id, entry)
  }

  onTestEnd(test: TestCase, result: TestResult): void {
    const project = test.parent?.project()?.name ?? ''
    this.cases.set(test.id, serialize(test, result, project))
  }

  onEnd(result: FullResult): void {
    try {
      fs.mkdirSync(path.dirname(this.target), { recursive: true })
      fs.writeFileSync(
        this.target,
        JSON.stringify(
          {
            generated_at: new Date().toISOString(),
            config: this.configFile,
            status: result.status,
            // Wall clock for THIS run, in seconds (per-case `duration` stays in
            // milliseconds, mirroring Playwright's own units).
            duration: result.duration / 1000,
            cases: Object.fromEntries(this.cases),
          },
          null,
          2,
        ),
      )
    } catch (err) {
      // Never fail the suite over the report channel.
      console.warn(`[testsuite-report] could not write ${this.target}: ${String(err)}`)
    }
  }
}

export default CollectJson