import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

async function focusAgent(page, agentRe) {
  const rows = page.getByRole('listitem', { name: /.+/ });
  const n = await rows.count();
  for (let i = 0; i < n; i++) {
    const row = rows.nth(i);
    const nm = (await row.textContent().then(t => (t || '').trim().slice(0, 24))) || '';
    if (/work/.test(nm)) continue;
    await row.click();
    await sleep(900);
    if (agentRe.test(await snapshot(page))) { console.log('matched row[' + i + '] ' + nm); return true; }
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
  if (!(await focusAgent(page, /🔒 slow /))) throw new Error('slow not found');
  await page.getByTestId('composer-input').locator('textarea').fill('第二次停止测试');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1000);
  const stop = page.getByRole('button', { name: '停止回复' });
  const vis = await stop.isVisible().catch(() => false);
  console.log('stop visible at T+1s: ' + vis);
  if (vis) {
    await stop.click();
    console.log('stop clicked at ~T+1s');
    await sleep(1500);
    await shot(page, 'b58_slow_stopped');
    const s = await snapshot(page);
    console.log('=== after stop ===');
    console.log(s.split('\n').filter(l => /paragraph|取消|中断|停止|已|Done|Working/.test(l)).slice(0, 16).join('\n'));
    // 跟发验证可用性
    await page.getByTestId('composer-input').locator('textarea').fill('停止后还能继续吗');
    await page.getByRole('button', { name: '发送' }).click();
    await sleep(6000);
    await shot(page, 'b59_slow_after_stop_continue');
    const s2 = await snapshot(page);
    console.log('=== continue ===');
    console.log(s2.split('\n').filter(l => /paragraph/.test(l)).slice(0, 14).join('\n'));
  } else {
    await shot(page, 'b58_slow_no_stop_btn');
    console.log('turn already finished at T+1s; stop button never appeared');
  }
}, 's55_slow_stop');
