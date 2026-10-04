import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  // 项目可能已注册（上次运行可能已提交）？先看状态
  console.log('=== ARIA SNAPSHOT (before) ===');
  const snap0 = await snapshot(page);
  console.log(snap0);
  if (snap0.includes('尚未注册项目')) {
    await page.getByRole('button', { name: '添加项目' }).click();
    await sleep(600);
    await page.getByRole('treeitem', { name: 'work' }).click();
    await sleep(500);
    // 找 dialog 内 enabled 的提交按钮：用 footer 区域可见性过滤
    const candidates = page.getByRole('button', { name: '添加项目' });
    const n = await candidates.count();
    console.log('add-project button count: ' + n);
    for (let i = 0; i < n; i++) {
      const b = candidates.nth(i);
      console.log(`btn[${i}] visible=${await b.isVisible()} disabled=${await b.isDisabled()}`);
    }
    // 点击 enabled 的那个
    for (let i = 0; i < n; i++) {
      const b = candidates.nth(i);
      if (await b.isEnabled()) {
        console.log('clicking btn[' + i + ']');
        await b.click();
        break;
      }
    }
    await sleep(1500);
  }
  console.log('=== ARIA SNAPSHOT (after) ===');
  console.log(await snapshot(page));
  await shot(page, 'b06_project_registered');
}, 's04b_register_project');
