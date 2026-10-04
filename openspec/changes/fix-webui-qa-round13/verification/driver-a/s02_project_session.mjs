import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  // 注册项目：点「添加项目」
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(1200);
  console.log('--- snapshot add-project dialog ---');
  console.log(await snapshot(page));
  await shot(page, 'a05_add_project_dialog');
}, 's02a_addproj_dialog');
console.log('DONE');
