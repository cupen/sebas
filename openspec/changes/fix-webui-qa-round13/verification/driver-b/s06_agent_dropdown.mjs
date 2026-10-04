import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(800);
  // 打开 Agent 下拉
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(600);
  console.log('=== ARIA SNAPSHOT (agent dropdown open) ===');
  console.log(await snapshot(page));
  await shot(page, 'b09_agent_dropdown');
}, 's06_agent_dropdown');
