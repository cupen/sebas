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
  // 逐个未命名会话找 slow
  const unnamed = page.getByRole('listitem', { name: '未命名会话', exact: true });
  const n = await unnamed.count();
  console.log('unnamed: ' + n);
  let slowIdx = -1;
  for (let i = 0; i < n; i++) {
    await unnamed.nth(i).click();
    await sleep(900);
    const h = await page.locator('main').innerText();
    const agent = h.split('\n').find(l => /thinking|claude|slow|empty|error|slash/.test(l)) || '';
    console.log(`[${i}] header agent hint: ` + agent.slice(0, 80));
    if (/slow/.test(agent)) { slowIdx = i; break; }
  }
  if (slowIdx < 0) throw new Error('slow session not found');
  console.log('slow at index ' + slowIdx);
  // 发消息触发流式
  await page.getByTestId('composer-input').locator('textarea').fill('流式测试');
  await page.getByRole('button', { name: '发送' }).click();
  await sleep(1200);
  await shot(page, 'b49_slow_streaming_1');
  const s1 = await snapshot(page);
  console.log('=== streaming state ===');
  console.log(s1.split('\n').filter(l => /停止|Working|Queued|paragraph|text:/.test(l)).slice(0, 20).join('\n'));
}, 's47_find_slow_stream');
