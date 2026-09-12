/**
 * Reusable dual-process (detached) sandbox fixture — harden-core-channel-deployment
 * task 5.4 and a DELIVERABLE for the sibling change cover-core-channel-test-gaps:
 * the B1 detached-approval journey reuses this fixture instead of rewriting
 * the harness.
 *
 * Topology (assembled by `invoke testsuite-webui-server` with
 * TESTSUITE_MODE=detached, tasks.py): a core process WITHOUT `--webui`
 * (`sebas core -c <scene>/config.toml`；router 只以独立进程运行，core 旗标里
 * 没有 router——unify-router-process-shape) plus a standalone
 * `sebas webui -c <scene>/config.toml` serving the dashboard. NO
 * SEBAS_CORE_SECRET is set anywhere: the core auto-arms (generates the key,
 * writes `<scene>/core.secret`) and clients discover the key from that file
 * on every connect attempt — so restarting the core with a fresh key
 * self-heals on the untouched webui side.
 *
 * Scene contract published by the harness:
 * - TESTSUITE_SCENE_FILE → the throwaway scene dir (config.toml, logs,
 *   state files inside);
 * - `<scene>/pids.json` → `{"core": pid, "webui": pid}`, updated whenever
 *   `startCore()` spawns a fresh core, so teardown always kills the live set.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { APIRequestContext } from '@playwright/test'

export const DETACHED_SCENE_FILE =
  process.env.TESTSUITE_SCENE_FILE ?? path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9897')

/** The throwaway scene dir (throws honestly if the harness didn't publish one). */
export function detachedSceneDir(): string {
  const scene = fs.readFileSync(DETACHED_SCENE_FILE, 'utf8').trim()
  if (!scene || !fs.existsSync(scene)) {
    throw new Error(`detached scene dir not found via TESTSUITE_SCENE_FILE=${DETACHED_SCENE_FILE}`)
  }
  return scene
}

/** Repo-built sebas binary (the same one the harness assembled). */
export function sebasBin(): string {
  const suffix = process.platform === 'win32' ? '.exe' : ''
  return path.resolve(import.meta.dirname, '../../../..', 'target', 'debug', `sebas${suffix}`)
}

/**
 * Env for a core (or any channel client) spawned against the scene: every
 * default that would fall back to the real `~/.sebas` is redirected into
 * the scene (SAME set the harness injects — SEBAS_PROJECTS_PATH /
 * SEBAS_WEBUI_AUTH_FILE / SEBAS_HOME included: the projects registry
 * defaults to `~/.sebas/projects.json`), and SEBAS_CORE_SECRET is REMOVED
 * so the auto-arm + discovery path is exercised, never the env shortcut.
 */
export function detachedCoreEnv(scene: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...process.env }
  delete env.SEBAS_CORE_SECRET
  env.SEBAS_HOME = scene
  env.SEBAS_STATE_DB = path.join(scene, 'sebas.db')
  env.SEBAS_STATE_FILE = path.join(scene, 'state.json')
  env.SEBAS_ROUTER_PROVIDER_OVERLAY = path.join(scene, 'providers.json')
  env.SEBAS_PROJECTS_PATH = path.join(scene, 'projects.json')
  env.SEBAS_WEBUI_AUTH_FILE = path.join(scene, 'webui-auth.json')
  return env
}

interface ScenePids {
  core?: number | null
  webui?: number | null
}

function readPids(scene: string): ScenePids {
  try {
    return JSON.parse(fs.readFileSync(path.join(scene, 'pids.json'), 'utf8')) as ScenePids
  } catch {
    return {}
  }
}

function writePids(scene: string, pids: ScenePids): void {
  fs.writeFileSync(path.join(scene, 'pids.json'), JSON.stringify(pids))
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/**
 * Truly dead = signal delivery fails (ESRCH) or — linux only — the process
 * is a zombie. Zombies matter here: the harness parent never reaps its core
 * child, so after SIGTERM `kill(pid, 0)` keeps succeeding on the zombie and
 * a naive wait would time out.
 */
export function isPidDead(pid: number): boolean {
  if (!alive(pid)) {
    return true
  }
  if (process.platform === 'linux') {
    try {
      const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8')
      const state = stat.slice(stat.lastIndexOf(')') + 2).trim().split(' ')[0]
      return state === 'Z'
    } catch {
      // /proc unreadable → fall through to the signal-based verdict.
    }
  }
  return false
}

/** Whether the scene currently tracks a live (non-zombie) core process. */
export function isCoreAlive(scene: string): boolean {
  const pid = readPids(scene).core
  return pid !== null && pid !== undefined && !isPidDead(pid)
}

/**
 * Stop the current core (SIGTERM — graceful exit removes the channel socket
 * and DUMPS state but keeps the secret file, design D5) and wait until the
 * pid is gone. Escalates to SIGKILL if the graceful exit does not complete
 * in time (the next start reclaims a stale socket file). Idempotent: a dead
 * core is a no-op.
 */
export async function stopCore(scene: string, timeoutMs = 20_000): Promise<void> {
  const pid = readPids(scene).core
  if (pid === null || pid === undefined || isPidDead(pid)) {
    writePids(scene, { ...readPids(scene), core: null })
    return
  }
  process.kill(pid, 'SIGTERM')
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (isPidDead(pid)) {
      writePids(scene, { ...readPids(scene), core: null })
      return
    }
    await new Promise((r) => setTimeout(r, 100))
  }
  try {
    process.kill(pid, 'SIGKILL')
  } catch {
    // Already gone between the check and the kill.
  }
  const hardDeadline = Date.now() + 5_000
  while (Date.now() < hardDeadline && !isPidDead(pid)) {
    await new Promise((r) => setTimeout(r, 100))
  }
  writePids(scene, { ...readPids(scene), core: null })
}

/**
 * Start a fresh core against the scene (cwd = scene so the config's relative
 * channel path resolves) and publish its pid. Resolves once the process is
 * running — pair with `waitForCoreReachability` for service-level readiness.
 */
export function startCore(scene: string): ChildProcess {
  const log = fs.openSync(path.join(scene, 'core.log'), 'a')
  const child = spawn(sebasBin(), ['core', '-c', path.join(scene, 'config.toml')], {
    cwd: scene,
    env: detachedCoreEnv(scene),
    stdio: ['ignore', log, log],
  })
  child.on('error', (e) => {
    throw new Error(`failed to spawn core from scene ${scene}: ${e.message}`)
  })
  writePids(scene, { ...readPids(scene), core: child.pid ?? null })
  return child
}

/** The pid of the currently tracked core (null when none is running). */
export function corePid(scene: string): number | null {
  return readPids(scene).core ?? null
}

export async function reachabilityOk(request: APIRequestContext): Promise<boolean | null> {
  try {
    const r = (await (await request.get('/api/summary')).json()) as {
      reachability?: { ok?: boolean }
    }
    return r.reachability?.ok !== false
  } catch {
    return null
  }
}

/**
 * Poll the webui `/api/summary` until the core channel reports the wanted
 * state (frontend polls on a 5s cadence, so give it room).
 */
export async function waitForCoreReachability(
  request: APIRequestContext,
  ok: boolean,
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    if ((await reachabilityOk(request)) === ok) {
      return
    }
    if (Date.now() > deadline) {
      throw new Error(`core reachability did not become ok=${ok} within ${timeoutMs}ms`)
    }
    await new Promise((r) => setTimeout(r, 500))
  }
}
