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
  await unnamed.nth(1).click(); // slow
  await sleep(1000);
  await page.getByTestId('composer-input').locator('textarea').fill('流式测试请慢速回答');
  await page.getByRole('button', { name: '发送' }).click();
  // 抓流式增量：T+2s、T+5s 各一张
  await sleep(2000);
  await shot(page, 'b50_stream_t2');
  const s1 = await snapshot(page);
  console.log('=== T+2s ===');
  console.log(s1.split('\n').filter(l => /停止|Working|Queued|paragraph|text:|Done/.test(l)).slice(0, 14).join('\n'));
  await sleep(3000);
  await shot(page, 'b51_stream_t5');
  const s2 = await snapshot(page);
  console.log('=== T+5s ===');
  console.log(s2.split('\n').filter(l => /停止|Working|paragraph|text:|Done/.test(l)).slice(0, 14).join('\n'));
}, 's50_stream_start');
