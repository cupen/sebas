/**
 * Sandbox scene access: the harness (`invoke testsuite-webui-server`, tasks.py)
 * publishes its throwaway sandbox dir at TESTSUITE_SCENE_FILE. Specs that need a
 * real directory inside the sandbox (project-add journey, folder-picker
 * navigation) read the pointer rather than guessing the mktemp path.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

export const SCENE_FILE =
  process.env.TESTSUITE_SCENE_FILE ?? path.join(os.tmpdir(), 'sebas-testsuite-webui-scene-9899')

/** The throwaway sandbox dir (throws honestly if the harness didn't publish one). */
export function sceneDir(): string {
  const scene = fs.readFileSync(SCENE_FILE, 'utf8').trim()
  if (!scene || !fs.existsSync(scene)) {
    throw new Error(`sandbox scene dir not found via TESTSUITE_SCENE_FILE=${SCENE_FILE}`)
  }
  return scene
}
