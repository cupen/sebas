// QA round 11 fix verification — v1: login silence / native chain / stop / lifecycle / notifications / logout
import { createRequire } from 'module';
const require = createRequire('D:/workbench/repos-ai/sebas/tests/testsuite-webui/');
const { chromium } = require('@playwright/test');

const BASE = 'http://127.0.0.1:9877';
const SHOTS = 'D:/workbench/repos-ai/sebas/openspec/changes/fix-webui-qa-round11/verification/shots';

const results = [];
function report(id, ok, detail = '') {
  results.push({ id, ok, detail });
  console.log(`${ok ? 'PASS' : 'FAIL'} ${id}${detail ? ' — ' + detail : ''}`);
}
async function shot(page, name) {
  await page.screenshot({ path: `${SHOTS}/${name}.png` });
}
async function dump(page, tag) {
  const snap = await page.locator('body').ariaSnapshot().catch(() => 'NO_SNAPSHOT');
  console.log(`ARIA[${tag}]: ${snap.slice(0, 2200)}`);
}
const consoleLog = [];
function watch(page, tag) {
  page.on('console', (m) => {
    if (m.type() === 'error') consoleLog.push({ tag, text: m.text() });
  });
  page.on('pageerror', (e) => consoleLog.push({ tag, text: 'pageerror: ' + e.message }));
  page.on('response', (r) => {
    if (r.status() >= 400) consoleLog.push({ tag, text: `HTTP ${r.status()} ${r.url()}` });
  });
}

const browser = await chromium.launch();
try {
  // ── V01: unauthenticated login page is console-silent (A-3/B-6) ──
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  watch(page, 'v');
  await page.goto(BASE + '/', { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(6000);
  await shot(page, 'v01_login_page');
  const anyErrs = consoleLog.filter((c) => c.tag === 'v');
  report('V01 login-page console silence', anyErrs.length === 0, `errors=${JSON.stringify(anyErrs.slice(0, 4))}`);

  // ── V02: login admin → workbench ──
  await page.locator('input').first().fill('admin');
  await page.locator('input[type="password"]').fill('admin');
  await page.getByRole('button', { name: /登录|login/i }).click();
  await page.waitForTimeout(2500);
  const connected = await page.getByText('核心已连接').first().isVisible().catch(() => false);
  await shot(page, 'v02_after_login');
  report('V02 admin login, core connected indicator', connected);

  // ── V03: create NATIVE session via rail + dialog ──
  await page.getByRole('button', { name: '在 proj-alpha 中新建会话' }).click();
  await page.waitForTimeout(800);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await page.getByRole('option', { name: /Native Kernel/ }).click();
  await page.waitForTimeout(400);
  await page.getByRole('button', { name: '创建会话' }).click();
  await page.waitForTimeout(2200);
  await shot(page, 'v03_after_create');
  const chip = await page.getByTestId('model-chip').textContent().catch(() => '');
  report('V03 native session created (model chip test/text)', /test\/text/.test(chip || ''), `chip=${chip}`);

  // ── V04: model → test/long, send, click stop EARLY (B-2 / 1.3) ──
  await page.getByTestId('model-chip').click();
  await page.waitForTimeout(500);
  await page.getByText('test/long', { exact: true }).first().click();
  await page.waitForTimeout(600);
  const composer = page.locator('textarea').last();
  await composer.fill('stream please');
  await composer.press('Enter');
  const stopSel = page.getByRole('button', { name: '停止回复' });
  let clicked = false;
  for (let i = 0; i < 40 && !clicked; i++) {
    if (await stopSel.isVisible().catch(() => false)) {
      await stopSel.click().catch(() => {});
      clicked = true;
    } else {
      await page.waitForTimeout(100);
    }
  }
  await shot(page, 'v04_streaming');
  report('V04 native streaming exposes stop control', clicked);
  if (clicked) {
    let notice = false;
    for (let i = 0; i < 20 && !notice; i++) {
      notice = (await page.getByText('回合已取消（操作者停止了本次回复）').count()) > 0;
      if (!notice) await page.waitForTimeout(300);
    }
    await shot(page, 'v04_after_stop');
    report('V04 stop settles with cancellation notice', notice);
  }
  // session usable after cancel: follow-up turn runs to its own summary
  await composer.fill('ping after cancel');
  await composer.press('Enter');
  let echoed = false;
  for (let i = 0; i < 30 && !echoed; i++) {
    echoed = (await page.getByText(/turn summary/).count()) >= 2;
    if (!echoed) await page.waitForTimeout(400);
  }
  await page.getByRole('button', { name: /跳到最新/ }).click().catch(() => {});
  await page.waitForTimeout(800);
  await shot(page, 'v04_after_cancel_ping');
  const followRan = (await page.getByText('ping after cancel').count()) >= 1;
  report('V04 session usable after cancel (follow-up turn ran)', followRan && echoed, `summary2=${echoed}`);

  // ── V05: lifecycle — title, done status, counters (B-1 / O-1) ──
  await page.waitForTimeout(1500);
  const titled = (await page.getByText('stream please').count()) > 0;
  const unnamed = (await page.getByText(/未命名会话/).count()) > 0;
  report('V05 native session auto-titled (rail)', titled && !unnamed, `titled=${titled} unnamed=${unnamed}`);
  await page.getByRole('link', { name: '历史' }).click();
  await page.waitForTimeout(1200);
  await shot(page, 'v05_history');
  report('V05 history shows titled session', (await page.getByText('stream please').count()) > 0);
  report('V05 session status advanced to Done', (await page.getByText('Done', { exact: true }).count()) > 0);
  const counterNode = (await page.getByText(/活跃/).count()) > 0 && (await page.getByText(/休眠/).count()) > 0;
  report('V05 lifecycle counters present', counterNode);
  if (!counterNode) await dump(page, 'v05_history');

  // ── V06: background completion notification (B-3) + 2nd turn re-arm ──
  const toastText = async () => {
    const items = page.locator('wa-toast-item');
    const n = await items.count();
    for (let i = 0; i < n; i++) {
      const t = await items.nth(i).innerText().catch(() => '');
      if (/回合已完成|回合失败/.test(t)) return t;
    }
    return '';
  };
  await page.getByText('stream please').first().click();
  await page.waitForTimeout(1200);
  await composer.fill('long stream two');
  await composer.press('Enter');
  await page.waitForTimeout(500);
  await page.getByRole('link', { name: '历史' }).click();
  let toast1 = '';
  for (let i = 0; i < 24 && !toast1; i++) { toast1 = await toastText(); if (!toast1) await page.waitForTimeout(500); }
  await shot(page, 'v06_toast_first');
  report('V06 1st background completion notifies (history page)', !!toast1, toast1.slice(0, 60));

  await page.getByText('stream please').first().click();
  await page.waitForTimeout(1200);
  await composer.fill('long stream three');
  await composer.press('Enter');
  await page.waitForTimeout(500);
  await page.getByRole('link', { name: '历史' }).click();
  let toast2 = '';
  for (let i = 0; i < 24 && !toast2; i++) { toast2 = await toastText(); if (!toast2) await page.waitForTimeout(500); }
  await shot(page, 'v06_toast_second');
  report('V06 2nd background completion notifies (re-arm)', !!toast2, toast2.slice(0, 60));

  // ── V07: logout → console stays silent (3.2 WS gate) ──
  const before = consoleLog.length;
  await page.getByRole('button', { name: /退出/ }).click();
  await page.waitForTimeout(2000);
  const onLogin = await page.locator('input').first().isVisible().catch(() => false);
  await page.waitForTimeout(5000);
  await shot(page, 'v07_after_logout');
  const wsAfter = consoleLog.slice(before).filter((c) => /websocket|authentication failed/i.test(c.text));
  report('V07 logout → login page, WS silent', onLogin && wsAfter.length === 0, `wsErrs=${wsAfter.length}`);

  await ctx.close();
} finally {
  await browser.close();
}
console.log('SUMMARY ' + JSON.stringify(results));
