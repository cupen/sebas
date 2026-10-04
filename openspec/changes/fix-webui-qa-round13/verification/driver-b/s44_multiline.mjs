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
  // 聚焦 claude agent 的未命名会话（最后一个）——先点侧栏最后一个未命名会话
  const items = page.getByRole('listitem').filter({ hasText: '未命名会话' });
  console.log('unnamed sessions: ' + (await items.count()));
  await items.last().click();
  await sleep(900);
  // 多行输入：line1 Shift+Enter line2，再 Enter 发送
  const box = page.getByTestId('composer-input').locator('textarea');
  await box.click();
  await box.pressSequentially('line1');
  await box.press('Shift+Enter');
  await box.pressSequentially('line2');
  await shot(page, 'b45_multiline_typed');
  const val = await box.inputValue();
  console.log('textarea value JSON: ' + JSON.stringify(val));
  await box.press('Enter');
  await sleep(2000);
  for (let i = 0; i < 10; i++) {
    const t = await page.locator('main').innerText();
    if (/hello world/.test(t)) break;
    await sleep(1000);
  }
  console.log('=== transcript ===');
  const t = await page.locator('main').innerText();
  console.log(t.split('\n').slice(0, 40).join('\n'));
  await shot(page, 'b46_multiline_sent');
}, 's44_multiline');
