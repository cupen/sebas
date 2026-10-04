import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

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
  const unnamed = page.getByRole('listitem', { name: '未命名会话', exact: true });
  const n = await unnamed.count();
  let found = false;
  for (let i = 0; i < n; i++) {
    await unnamed.nth(i).click();
    await sleep(900);
    if (/🔒 claude /.test(await snapshot(page))) { found = true; break; }
  }
  if (!found) throw new Error('claude session not found');
  await page.getByTestId('composer-input').locator('textarea').fill('drip');
  await page.getByRole('button', { name: '发送' }).click();
  // 每 1.5s 采样一次正文长度，共 5 次，验证增量
  for (let i = 0; i < 5; i++) {
    await sleep(1500);
    const s = await snapshot(page);
    const paras = s.split('\n').filter(l => /paragraph/.test(l)).length;
    const chars = (s.match(/drip|滴/g) || []).length;
    console.log(`T+${(i + 1) * 1.5}s paragraphs=${paras}`);
    if (i === 1) await shot(page, 'b53_drip_mid_1');
    if (i === 3) await shot(page, 'b54_drip_mid_2');
  }
  // 若仍在流式（停止按钮在）→ 点停止
  const stop = page.getByRole('button', { name: '停止回复' });
  if (await stop.isVisible().catch(() => false)) {
    console.log('stopping mid-drip...');
    await stop.click();
    await sleep(1500);
    await shot(page, 'b55_drip_stopped');
    const s = await snapshot(page);
    console.log('=== after stop ===');
    console.log(s.split('\n').filter(l => /取消|中断|停止|paragraph|Done|已/.test(l)).slice(0, 16).join('\n'));
  } else {
    console.log('drip finished before stop; sampling tail');
    await shot(page, 'b55_drip_finished');
    const s = await snapshot(page);
    console.log(s.split('\n').filter(l => /paragraph/.test(l)).slice(0, 14).join('\n'));
  }
}, 's52_drip_stop');
