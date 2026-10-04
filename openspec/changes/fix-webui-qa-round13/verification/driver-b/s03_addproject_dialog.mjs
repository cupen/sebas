import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(800);
  console.log('=== ARIA SNAPSHOT (add project dialog) ===');
  console.log(await snapshot(page));
  await shot(page, 'b04_add_project_dialog');
}, 's03_addproject_dialog');
