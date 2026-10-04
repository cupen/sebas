import { chromium } from 'file:///D:/workbench/repos-ai/sebas/tests/testsuite-webui/node_modules/@playwright/test/index.mjs';
import fs from 'node:fs';
import path from 'node:path';

export const REPO = 'D:/workbench/repos-ai/sebas';
export const SHOTS = path.join(REPO, 'openspec/changes/fix-webui-qa-round13/verification/shots');
export const DRIVER = path.join(REPO, 'openspec/changes/fix-webui-qa-round13/verification/driver-a');
export const PROFILE = path.join(REPO, 'target/qa-r13-a-profile');
export const BASE = 'http://127.0.0.1:9877';
export const CONSOLE_LOG = path.join(DRIVER, 'console-a.jsonl');

fs.mkdirSync(SHOTS, { recursive: true });
fs.mkdirSync(path.dirname(CONSOLE_LOG), { recursive: true });

export function appendLog(obj) {
  fs.appendFileSync(CONSOLE_LOG, JSON.stringify({ ts: new Date().toISOString(), ...obj }) + '\n');
}

export async function shot(page, name) {
  const p = path.join(SHOTS, name.endsWith('.png') ? name : name + '.png');
  await page.screenshot({ path: p, fullPage: false });
  console.log('SHOT ' + p);
  return p;
}

export async function snapshot(page) {
  return page.locator('body').ariaSnapshot();
}

export async function withPage(fn, label = 'step') {
  const ctx = await chromium.launchPersistentContext(PROFILE, {
    headless: true,
    viewport: { width: 1440, height: 900 },
    locale: 'zh-CN',
  });
  const page = ctx.pages()[0] || (await ctx.newPage());
  page.setDefaultTimeout(8000);
  const errors = [];
  page.on('pageerror', (e) => {
    const t = String(e);
    errors.push('pageerror: ' + t);
    appendLog({ label, kind: 'pageerror', text: t, url: page.url() });
  });
  page.on('console', (m) => {
    if (m.type() === 'error') {
      const t = m.text();
      errors.push('console.error: ' + t);
      appendLog({ label, kind: 'console.error', text: t, url: page.url() });
    }
  });
  page.on('response', (r) => {
    if (r.status() >= 400) {
      appendLog({ label, kind: 'http', status: r.status(), url: r.url() });
    }
  });
  page.on('requestfailed', (r) => {
    appendLog({ label, kind: 'requestfailed', url: r.url(), text: r.failure()?.errorText });
  });
  let fnErr = null;
  try {
    await fn(page, ctx);
  } catch (e) {
    fnErr = e;
    appendLog({ label, kind: 'script-error', text: String(e?.stack || e) });
  } finally {
    await ctx.close().catch(() => {});
  }
  if (fnErr) throw fnErr;
  return errors;
}

export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }

// 等 toast 出现并在同一脚本内截图（瞬时态三连：before → action → wait → after 由调用方组合）
export async function waitToast(page, textRe, timeout = 6000) {
  try {
    await page.getByText(textRe).first().waitFor({ state: 'visible', timeout });
    return true;
  } catch {
    return false;
  }
}
