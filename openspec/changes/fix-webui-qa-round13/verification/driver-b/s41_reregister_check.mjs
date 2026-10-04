import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1500);
  const snap0 = await snapshot(page);
  if (snap0.includes('登录以继续')) {
    await page.getByRole('textbox', { name: '用户名' }).fill('admin');
    await page.getByRole('textbox', { name: '密码' }).fill('admin');
    await page.getByRole('button', { name: '登录' }).click();
    await sleep(2000);
  }
  // 重注册 work 项目
  if ((await snapshot(page)).includes('尚未注册项目')) {
    await page.getByRole('button', { name: '添加项目' }).first().click();
    await sleep(700);
    await page.getByRole('treeitem', { name: 'work' }).click();
    await sleep(500);
    await page.locator('wa-dialog').getByRole('button', { name: '添加项目' }).click();
    await sleep(1500);
  }
  // 查 provider 幸存
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await sleep(1200);
  await shot(page, 'b42_settings_models_after_restart');
  const modalText = await page.evaluate(() => {
    const m = document.querySelector('sebas-settings-modal');
    return m ? m.textContent.slice(0, 800) : 'no modal';
  });
  console.log(modalText);
}, 's41_reregister_check');
