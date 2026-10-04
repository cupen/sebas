import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (!s0.includes('退出 (member')) {
    if (s0.includes('退出登录')) { await page.getByRole('button', { name: '退出登录' }).click(); await sleep(1800); }
    await page.locator('input[type="text"]').first().fill('member');
    await page.locator('input[type="password"]').first().fill('member');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }
  console.log('态: ' + (await snapshot(page)).match(/退出 \(([^\)]+)\)/)?.[1]);

  // member GUI 发消息（聚焦 hello 会话）
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1500);
  const tb = page.locator('textarea').first();
  await tb.fill('member gui probe');
  await page.getByRole('button', { name: '发送' }).click();
  const ok = await page.getByText('hello world').first().waitFor({ state: 'visible', timeout: 30000 }).then(() => true).catch(() => false);
  console.log('member GUI 回合完成: ' + ok);
  await shot(page, 'a105_member_gui_turn');

  // member 建会话（GUI 完整路径）
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('button', { name: '创建会话' }).click();
  await sleep(1500);
  const s1 = await snapshot(page);
  console.log('member GUI 建会话: ' + (s1.includes('未命名会话') ? 'OK' : '?'));
  await shot(page, 'a106_member_created_session');
}, 's23_member_gui');
console.log('DONE');
