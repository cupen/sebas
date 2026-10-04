import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function focusAgent(page, agentRe) {
  const rows = page.getByRole('listitem', { name: /.+/ });
  const n = await rows.count();
  console.log('rows: ' + n);
  for (let i = 0; i < n; i++) {
    const row = rows.nth(i);
    const nm = await row.getAttribute('aria-label').catch(() => null) || await row.textContent().then(t => (t || '').trim().slice(0, 24));
    if (/work/.test(nm)) continue;
    await row.click();
    await sleep(900);
    const s = await snapshot(page);
    if (agentRe.test(s)) { console.log('matched row[' + i + '] ' + nm); return true; }
  }
  return false;
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1500);
  const snap0 = await snapshot(page);
  if (snap0.includes('登录以继续')) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  if (!(await focusAgent(page, /🔒 claude /))) throw new Error('claude session not found');
  await page.getByTestId('composer-input').locator('textarea').fill('stream');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(800);
  const stop = page.getByRole('button', { name: '停止回复' });
  console.log('stop visible at T+0.8s: ' + (await stop.isVisible().catch(() => false)));
  if (await stop.isVisible().catch(() => false)) await stop.click();
  await sleep(2000);
  await shot(page, 'b56_stream_stopped');
  const s = await snapshot(page);
  console.log('=== after stop ===');
  console.log(s.split('\n').filter(l => /paragraph|取消|中断|停止|Done|Working|已/.test(l)).slice(0, 16).join('\n'));
  await page.getByTestId('composer-input').locator('textarea').fill('stop 后继续');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(2500);
  for (let i = 0; i < 10; i++) {
    const t = await snapshot(page);
    if (/hello world/.test(t)) break;
    await sleep(1000);
  }
  await shot(page, 'b57_after_stop_continue');
  const s2 = await snapshot(page);
  console.log('=== continue after stop ===');
  console.log(s2.split('\n').filter(l => /paragraph/.test(l)).slice(0, 12).join('\n'));
}, 's54_stream_stop_v2');
