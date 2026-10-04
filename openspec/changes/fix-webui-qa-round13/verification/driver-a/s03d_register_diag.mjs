import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(1000);
  await page.getByRole('treeitem', { name: 'work' }).click();
  await sleep(2500); // 等 path 自动填充动画结束

  const allBtns = await page.locator('button').allTextContents();
  console.log('所有按钮文本: ' + JSON.stringify(allBtns.map(t => t.trim())));
  console.log('dialog 数: ' + await page.locator('[role="dialog"]').count());
  console.log('path: ' + JSON.stringify(await page.getByRole('textbox', { name: '项目路径' }).inputValue()));

  // 提交：dialog 内蓝色按钮（nav 里那个在 dialog 外），用 tree 之后的第一个
  const submit = page.locator('button:has-text("添加项目")').last();
  console.log('submit count: ' + await submit.count());
  await submit.click({ timeout: 10000 });
  await sleep(2000);
  const snap = await snapshot(page);
  console.log(snap.slice(0, 1800));
  await shot(page, 'a07_project_registered');
}, 's03d_register_diag');
console.log('DONE');
