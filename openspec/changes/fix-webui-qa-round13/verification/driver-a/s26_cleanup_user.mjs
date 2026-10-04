import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  const s0 = await snapshot(page);
  if (!s0.includes('退出 (admin')) {
    if (s0.includes('退出登录')) { await page.getByRole('button', { name: '退出登录' }).click(); await sleep(1800); }
    await page.locator('input[type="text"]').first().fill('admin');
    await page.locator('input[type="password"]').first().fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2500);
  }
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('用户', { exact: true }).first().click();
  await sleep(1000);
  // 删除 qa-live（最后一行 🗑）
  const delBtns = page.getByRole('button', { name: '🗑' });
  console.log('🗑 数: ' + await delBtns.count());
  await delBtns.last().click();
  await sleep(800);
  await shot(page, 'a122_user_delete_confirm');
  await page.getByRole('button', { name: /^删除|确认$/ }).last().click();
  await sleep(1500);
  const d = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('删除后 qa-live 在列表: ' + d.includes('qa-live'));
  console.log('member/viewer 仍在: ' + (d.includes('member member') && d.includes('viewer viewer')));
  await shot(page, 'a123_user_deleted');
}, 's26_cleanup');
console.log('DONE');
