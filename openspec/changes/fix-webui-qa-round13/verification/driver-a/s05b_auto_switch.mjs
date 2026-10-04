import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 自定义 combobox：点击展开 listbox
  const combo = page.getByRole('combobox').last();
  await combo.click();
  await sleep(800);
  console.log('--- listbox 展开 ---');
  console.log((await snapshot(page)).slice(0, 3400));
  await shot(page, 'a12_perm_menu_open');
}, 's05b_menu');
console.log('DONE');
