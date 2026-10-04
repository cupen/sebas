import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(800);
  await page.getByRole('treeitem', { name: 'work' }).click();
  await sleep(600);
  // dialog 内的提交按钮：用文本定位取最后一个
  const btns = page.locator('button', { hasText: '添加项目' });
  console.log('添加项目按钮数: ' + await btns.count());
  const submit = btns.last();
  console.log('disabled? ' + await submit.isDisabled().catch(e => 'err ' + e));
  console.log('path input: ' + JSON.stringify(await page.getByRole('textbox', { name: '项目路径' }).inputValue().catch(() => null)));
  await submit.click();
  await sleep(1800);
  console.log('--- snapshot after register ---');
  const snap = await snapshot(page);
  console.log(snap.slice(0, 2200));
  await shot(page, 'a07_project_registered');
}, 's03b_register');
console.log('DONE');
