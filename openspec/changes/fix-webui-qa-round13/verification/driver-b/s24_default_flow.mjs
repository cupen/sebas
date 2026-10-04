import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  // 上次状态：设置面板+默认对话框可能没留痕（每次新 context，重新来）
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1500);
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await sleep(1200);
  await page.mouse.click(988, 287);
  await sleep(800);
  // 设为默认按钮
  await page.getByRole('button', { name: '设为默认', exact: true }).click();
  await sleep(1000);
  await shot(page, 'b19_default_set2');
  // 关闭设置面板：先 Esc 关可能的子 dialog，再点 ×
  await page.keyboard.press('Escape');
  await sleep(400);
  const closeBtn = page.getByRole('button', { name: '关闭设置' });
  if (await closeBtn.isVisible().catch(() => false)) {
    await closeBtn.click();
  } else {
    await page.mouse.click(1073, 157);
  }
  await sleep(800);
  // 新建会话对话框
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(500);
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|combobox|dialog|heading/.test(l)).join('\n'));
  await shot(page, 'b20_agent_dropdown_default');
}, 's24_default_flow');
