import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import type { FullResult, Reporter } from '@playwright/test/reporter'

/**
 * Sandbox scene lifecycle, driven by the authoritative run status.
 *
 * The harness (`invoke testsuite-webui-server`, tasks.py) publishes its throwaway
 * sandbox dir at TESTSUITE_SCENE_FILE. This reporter owns the keep/clean decision because
 * it knows `FullResult.status`; it runs when the run finishes after the
 * webServer stops, so the scene is fully settled here.
 *
 *   - failure, or TESTSUITE_KEEP=1 → keep the scene (debug flow) and print the path;
 *   - success (no TESTSUITE_KEEP) → delete the scene + pointer.
 *
 * `globalTeardown` is a no-op fallback; this reporter is authoritative.
 */
const sceneFileDefault = () =>
  path.join(os.tmpdir(), process.env.TESTSUITE_SCENE_FILE?.endsWith('-9898') ? 'sebas-testsuite-webui-scene-9898' : 'sebas-testsuite-webui-scene-9899')

class KeepOnFail implements Reporter {
  onEnd(result: FullResult): void {
    const sceneFile = process.env.TESTSUITE_SCENE_FILE ?? sceneFileDefault()
    let scene = ''
    try {
      scene = fs.readFileSync(sceneFile, 'utf8').trim()
    } catch {
      return // no pointer — nothing to manage
    }
    if (!scene || !fs.existsSync(scene)) {
      try { fs.rmSync(sceneFile, { force: true }) } catch { /* best effort */ }
      return
    }

    const failed = result.status !== 'passed'
    const keep = failed || process.env.TESTSUITE_KEEP === '1'
    if (keep) {
      // record the reason, then preserve (never delete on failure).
      try { fs.writeFileSync(path.join(scene, '.tests-failed'), `${new Date().toISOString()}\n`) } catch { /* best effort */ }
      console.log(`\n[testsuite] tests ${failed ? 'failed' : 'kept via TESTSUITE_KEEP'} — sandbox scene kept at: ${scene}`)
      console.log(`[testsuite] backend log: ${scene}/core.log ; reuse with TESTSUITE_REUSE=1`)
    } else {
      try {
        fs.rmSync(scene, { recursive: true, force: true })
        fs.rmSync(sceneFile, { force: true })
        console.log(`[testsuite] sandbox cleaned: ${scene}`)
      } catch {
        // best effort — the sandbox is throwaway
      }
    }
  }
}

export default KeepOnFail
