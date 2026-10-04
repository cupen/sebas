import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(600);
  await page.getByRole('treeitem', { name: 'work' }).click();
  await sleep(500);
  await shot(page, 'b05_add_project_selected');
  // 添加项目按钮（dialog 内）此时应 enabled
  const dlg = page.getByRole('dialog');
  const addBtn = dlg.getByRole('button', { name: '添加项目' });
  console.log('add button disabled? ' + (await addBtn.isDisabled()));
  await addBtn.click();
  await sleep(1500);
  console.log('=== ARIA SNAPSHOT (after register) ===');
  console.log(await snapshot(page));
  await shot(page, 'b06_project_registered');
}, 's04_register_project');
