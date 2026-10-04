import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  // 注册项目 work
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(800);
  await page.getByRole('treeitem', { name: 'work' }).click();
  await sleep(500);
  await shot(page, 'a06_add_project_work_selected');
  const dialogAdd = page.getByRole('dialog').getByRole('button', { name: '添加项目' });
  await dialogAdd.click();
  await sleep(1500);
  console.log('--- snapshot after register ---');
  console.log(await snapshot(page));
  await shot(page, 'a07_project_registered');
}, 's03a_register');
console.log('DONE');
