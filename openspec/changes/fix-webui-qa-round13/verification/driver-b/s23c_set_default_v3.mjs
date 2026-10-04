import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1500);
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await sleep(1200);
  // fake 行的星标（默认 provider）——来自 b19a 截图坐标
  await page.mouse.click(988, 287);
  await sleep(1200);
  await shot(page, 'b19_default_set');
  // 关闭设置（右上 ×）
  await page.mouse.click(1073, 157);
  await sleep(800);
  // 打开新建会话
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(500);
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|combobox|dialog|heading/.test(l)).join('\n'));
  await shot(page, 'b20_agent_dropdown_default');
}, 's23c_set_default_v3');
