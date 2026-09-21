/**
 * Single-process core process control (core --webui topologies).
 *
 * The freeze/dead journeys drive the HARNESS-SPAWNED core process itself:
 * SIGSTOP freezes it (process alive, TCP established, HTTP + WS both silent —
 * the "hung upstream" shape no route-injection can reproduce), SIGCONT thaws
 * it, SIGKILL kills it outright. The harness publishes the pids at
 * `<scene>/pids.json` (`{"core": …, "router": …}`) for exactly this purpose;
 * TESTSUITE_ALLOW_CORE_DEATH=1 tells the harness to keep serving the scene
 * after the core process exits (the dead-core journey would otherwise trip
 * the harness's child-death teardown mid-test).
 *
 * POSIX only by design: SIGSTOP/SIGCONT have no Windows equivalent and the
 * browser suite is Linux-first (README 平台适配) — Windows honestly refuses.
 */
import fs from 'node:fs'
import path from 'node:path'
import { sceneDir } from './scene.js'

interface SandboxPids {
  core?: number | null
  router?: number | null
}

function readPids(scene: string): SandboxPids {
  try {
    return JSON.parse(fs.readFileSync(path.join(scene, 'pids.json'), 'utf8')) as SandboxPids
  } catch {
    return {}
  }
}

function pidAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function requirePosix(): void {
  if (process.platform === 'win32') {
    throw new Error('core process freeze/kill journeys need POSIX signals (linux-first suite)')
  }
}

/** The harness-spawned core (core --webui) pid; throws honestly when absent. */
export function sandboxCorePid(scene: string = sceneDir()): number {
  const pid = readPids(scene).core
  if (pid === null || pid === undefined || !pidAlive(pid)) {
    throw new Error(`sandbox core pid not found/alive via ${path.join(scene, 'pids.json')}`)
  }
  return pid
}

/** Freeze the core: alive but silent (SIGSTOP). Idempotent for our purposes. */
export function freezeCore(scene: string = sceneDir()): void {
  requirePosix()
  process.kill(sandboxCorePid(scene), 'SIGSTOP')
}

/** Thaw the frozen core (SIGCONT) and wait until the pid accepts signals again. */
export function thawCore(scene: string = sceneDir()): void {
  requirePosix()
  process.kill(sandboxCorePid(scene), 'SIGCONT')
}

/**
 * Kill the core outright (SIGKILL — instant, no graceful dump; the UI shape
 * under test is "server gone", identical for SIGTERM, and SIGKILL is
 * deterministic). Resolves when the pid is gone or a zombie (the harness
 * parent reaps on its own teardown).
 */
export async function killCoreHard(scene: string = sceneDir()): Promise<void> {
  requirePosix()
  const pid = sandboxCorePid(scene)
  process.kill(pid, 'SIGKILL')
  const deadline = Date.now() + 5_000
  for (;;) {
    try {
      const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8')
      const state = stat.slice(stat.lastIndexOf(')') + 2).trim().split(' ')[0]
      if (state === 'Z') return
    } catch {
      return // ESRCH — fully gone.
    }
    if (Date.now() > deadline) {
      throw new Error(`core pid ${pid} still alive after SIGKILL`)
    }
    await new Promise((r) => setTimeout(r, 50))
  }
}
