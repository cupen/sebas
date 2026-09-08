/**
 * Dual-process sandbox fixture (harden-core-channel-deployment §5.4).
 *
 * Generic process control for the detached assembly (`invoke
 * testsuite-webui-server-detached`: real `sebas core` + standalone
 * `sebas webui`, one throwaway config). The harness publishes `proc.json`
 * in the scene dir (spawn command + env + pids); these helpers drive the
 * core lifecycle from inside a browser journey. DELIVERABLE: later detached
 * e2es (approval flow) reuse this module unchanged — journey-specific
 * assertions stay in the specs, never here.
 */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import type { APIRequestContext } from '@playwright/test'
import { getSummary } from './api'
import { sceneDir } from './scene'

export interface DualProc {
  coreCmd: string[]
  coreEnv: Record<string, string>
  corePid: number
  webuiPid: number
}

interface ProcFile {
  core_cmd: string[]
  core_env: Record<string, string>
  core_pid: number
  webui_cmd: string[]
  webui_pid: number
}

function procFile(): string {
  return path.join(sceneDir(), 'proc.json')
}

/** Spawn record published by the harness (throws honestly when absent). */
export function readDualProc(): DualProc {
  const raw = fs.readFileSync(procFile(), 'utf8')
  const d = JSON.parse(raw) as ProcFile
  return { coreCmd: d.core_cmd, coreEnv: d.core_env, corePid: d.core_pid, webuiPid: d.webui_pid }
}

function writeCorePid(pid: number): void {
  const d = JSON.parse(fs.readFileSync(procFile(), 'utf8')) as ProcFile
  d.core_pid = pid
  fs.writeFileSync(procFile(), JSON.stringify(d))
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

/** Poll until `probe` returns non-null (bounded; throws with context on timeout). */
async function poll<T>(
  what: string,
  timeoutMs: number,
  probe: () => Promise<T | null>,
): Promise<T> {
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const v = await probe()
    if (v !== null) return v
    if (Date.now() >= deadline) throw new Error(`timeout waiting for ${what}`)
    await new Promise((r) => setTimeout(r, 250))
  }
}

/** `/api/summary` reachability ok (the assembly starts connected). */
export async function waitReachable(request: APIRequestContext, timeoutMs = 30_000) {
  return poll('core reachability ok', timeoutMs, async () => {
    const s = await getSummary(request)
    return s.reachability.ok ? s : null
  })
}

/** Reachability flipped false with a non-empty cause; returns the cause. */
export async function waitUnreachable(request: APIRequestContext, timeoutMs = 20_000) {
  return poll('reachability flip to unreachable with cause', timeoutMs, async () => {
    const s = await getSummary(request).catch(() => null)
    if (s && !s.reachability.ok && s.reachability.cause) return s.reachability.cause
    return null
  })
}

/** SIGKILL the core child (supervisor-crash path from the webui's view). */
export async function killCore(timeoutMs = 10_000): Promise<void> {
  const { corePid } = readDualProc()
  if (!alive(corePid)) throw new Error(`core pid ${corePid} already dead before killCore`)
  process.kill(corePid, 'SIGKILL')
  await poll(`core pid ${corePid} to exit`, timeoutMs, async () =>
    alive(corePid) ? null : true,
  )
}

/**
 * Respawn the core with the harness-recorded command + env (same config, so
 * the same channel socket and secret discovery apply). Updates `proc.json`.
 */
export async function startCore(timeoutMs = 30_000): Promise<number> {
  const { coreCmd, coreEnv } = readDualProc()
  const log = fs.openSync(path.join(sceneDir(), 'core-restarted.log'), 'a')
  const child = spawn(coreCmd[0], coreCmd.slice(1), {
    env: coreEnv,
    detached: true,
    stdio: ['ignore', log, log],
    windowsHide: true,
  })
  child.unref()
  if (child.pid === undefined) throw new Error('respawned core has no pid')
  writeCorePid(child.pid)
  const pid = child.pid
  // The old socket file may linger from the kill; the respawned core must
  // own the channel before the journey proceeds.
  await poll(`respawned core pid ${pid} to stay alive`, timeoutMs, async () =>
    alive(pid) ? true : null,
  )
  return pid
}
