import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, '用户');
  // 建 qa-live
  await page.getByRole('button', { name: '＋ 新建用户' }).click();
  await sleep(900);
  console.log('--- 新建用户表单 ---');
  const inv = page.locator('wa-input, wa-select, input, select');
  const n = await inv.count();
  for (let i = 0; i < n; i++) {
    const info = await inv.nth(i).evaluate(e => e.tagName + '|label=' + (e.getAttribute('label') || '') + '|ph=' + (e.getAttribute('placeholder') || '') + '|testid=' + (e.getAttribute('data-testid') || '')).catch(() => '');
    if (info) console.log(i + ': ' + info);
  }
  await shot(page, 'a77_user_form');
}, 's20b_user_form');
console.log('DONE');
