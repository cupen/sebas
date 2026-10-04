import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  const snap0 = await snapshot(page);
  if (snap0.includes('尚未注册项目')) {
    await page.getByRole('button', { name: '添加项目' }).first().click();
    await sleep(600);
    await page.getByRole('treeitem', { name: 'work' }).click();
    await sleep(500);
    const dlgBtn = page.locator('wa-dialog').getByRole('button', { name: '添加项目' });
    console.log('dialog submit count: ' + (await dlgBtn.count()));
    console.log('dialog submit disabled: ' + (await dlgBtn.isDisabled()));
    await dlgBtn.click();
    await sleep(1500);
  }
  console.log('=== ARIA SNAPSHOT (after register) ===');
  console.log(await snapshot(page));
  await shot(page, 'b06_project_registered');
}, 's04c_register_project');
