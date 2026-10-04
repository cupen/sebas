import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1200);
  await page.getByRole('listitem').filter({ hasText: 'hello' }).first().click();
  await sleep(800);
  // 模型 chip
  await page.getByRole('button', { name: 'test' }).click();
  await sleep(700);
  const snap = await snapshot(page);
  console.log('=== after model chip click ===');
  console.log(snap.split('\n').filter(l => /option|listbox|combobox|menu|menuitem|模型/.test(l)).join('\n'));
  await shot(page, 'b27_model_chip_menu');
}, 's29_model_chip');
