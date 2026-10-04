import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('button', { name: '项目操作：downloads' }).click();
  await sleep(700);
  await page.getByRole('menuitem', { name: '移除项目' }).click();
  await sleep(800);
  await shot(page, 'a74_project_remove_confirm');
  const s0 = await snapshot(page);
  const di = s0.indexOf('移除');
  console.log('--- 移除确认 ---');
  console.log(s0.slice(Math.max(0, di - 200), di + 500));
  // 确认移除
  const ok = page.getByRole('button', { name: /移除|删除|确认/ }).last();
  await ok.click();
  await sleep(1500);
  const s1 = await snapshot(page);
  console.log('移除后 downloads 仍在树: ' + s1.includes('downloads local'));
  await shot(page, 'a75_project_removed');
}, 's19b_remove');
console.log('DONE');
