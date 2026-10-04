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
    await sleep(1000);
    const s = await snapshot(page);
    if (/🔒 slow /.test(s)) { found = true; console.log('slow at [' + i + ']'); break; }
  }
  if (!found) throw new Error('slow not found');
  await page.getByTestId('composer-input').locator('textarea').fill('流式测试请慢速回答');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(2500);
  await shot(page, 'b50_stream_t2');
  const s1 = await snapshot(page);
  console.log('=== T+2.5s ===');
  console.log(s1.split('\n').filter(l => /停止|Working|Queued|paragraph|●|Done/.test(l)).slice(0, 12).join('\n'));
  await sleep(3000);
  await shot(page, 'b51_stream_t5');
  const s2 = await snapshot(page);
  console.log('=== T+5.5s ===');
  console.log(s2.split('\n').filter(l => /停止|Working|paragraph|●|Done/.test(l)).slice(0, 12).join('\n'));
  // 流式中点停止
  const stop = page.getByRole('button', { name: '停止回复' });
  console.log('stop visible: ' + (await stop.isVisible().catch(() => false)));
  if (await stop.isVisible().catch(() => false)) {
    await stop.click();
    await sleep(1500);
    await shot(page, 'b52_stream_stopped');
    const s3 = await snapshot(page);
    console.log('=== after stop ===');
    console.log(s3.split('\n').filter(l => /取消|停止|停止回复|paragraph|Done|Working|已中断|中断/.test(l)).slice(0, 14).join('\n'));
  }
}, 's51_slow_stream_stop');
