import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(1000);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(600);
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|combobox/.test(l)).join('\n'));
  await shot(page, 'b22_native_kernel_state_fresh');
  // 关掉下拉
  await page.keyboard.press('Escape');
  await sleep(300);
  await page.keyboard.press('Escape');
  await sleep(300);
}, 's26_native_fresh_check');
