import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  // 聚焦 hello 会话
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 权限模式下拉选项
  const combo = page.getByRole('combobox').last();
  const opts = await combo.locator('option').allTextContents();
  console.log('权限模式选项: ' + JSON.stringify(opts.map(t => t.trim())));

  // 切 Auto
  await combo.selectOption({ index: opts.findIndex(o => o.includes('Auto')) >= 0 ? opts.findIndex(o => o.includes('Auto')) : 0 });
  await sleep(1200);
  console.log('--- 切 Auto 后 ---');
  console.log((await snapshot(page)).slice(0, 3000));
  await shot(page, 'a12_switched_auto');
}, 's05a_auto_switch');
console.log('DONE');
