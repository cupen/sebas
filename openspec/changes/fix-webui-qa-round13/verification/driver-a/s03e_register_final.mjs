import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(1000);
  await page.getByRole('treeitem', { name: 'work' }).click();
  await sleep(2500); // 等 path 自动填充动画结束

  // dialog 内提交按钮 = 同名按钮中的最后一个（nav 一个 + dialog 一个）
  const submit = page.getByRole('button', { name: '添加项目' }).last();
  console.log('submit count: ' + await submit.count());
  await submit.click({ timeout: 10000 });
  await sleep(2000);
  const snap = await snapshot(page);
  console.log(snap.slice(0, 2000));
  await shot(page, 'a07_project_registered');
}, 's03e_register');
console.log('DONE');
