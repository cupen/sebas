import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 切 Auto
  const combo = page.getByRole('combobox').last();
  await combo.click();
  await sleep(600);
  await page.getByRole('option', { name: 'Auto · 自动执行' }).click();
  await sleep(1500);
  const s1 = await snapshot(page);
  console.log('--- 切 Auto 后（找系统卡/徽章）---');
  console.log(s1.slice(0, 3200));
  await shot(page, 'a13_switched_auto');
}, 's05c_auto');
console.log('DONE');
