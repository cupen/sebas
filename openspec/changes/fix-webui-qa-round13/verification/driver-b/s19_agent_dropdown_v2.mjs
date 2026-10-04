import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(800);
  await page.getByRole('combobox', { name: 'Agent' }).click();
  await sleep(500);
  console.log('=== ARIA (agent dropdown) ===');
  const snap = await snapshot(page);
  console.log(snap.split('\n').filter(l => /option|listbox|combobox|dialog|text:|status/.test(l)).join('\n'));
  await shot(page, 'b18_agent_dropdown_with_provider');
}, 's19_agent_dropdown_v2');
