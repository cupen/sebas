import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

async function fillField(page, testid, value) {
  await page.getByTestId(testid).locator('input, textarea').first().fill(value);
}

await withPage(async (page) => {
  await openSettingsTab(page, 'Agent');
  await page.getByRole('button', { name: '＋ 新建 agent' }).click();
  await sleep(800);
  await fillField(page, 'agent-form-id', 'claude-think');
  await fillField(page, 'agent-form-display', 'claude-think（thinking 场景）');
  // 形态 WA-SELECT：看当前值
  const shape = page.getByTestId('agent-form-shape');
  console.log('形态当前值: ' + JSON.stringify(await shape.textContent().catch(() => '')));
  await fillField(page, 'agent-form-path', 'D:/workbench/repos-ai/sebas/target/debug/fake-claude-qa13.exe');
  await fillField(page, 'agent-form-args', '--scenario thinking');
  await sleep(300);
  await shot(page, 'a50_agent_form_filled');
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1500);
  console.log('--- 保存后 Agent 列表 ---');
  console.log(await page.locator('[role="dialog"]').first().ariaSnapshot());
  await shot(page, 'a51_agent_saved');
}, 's15c_agent_save');
console.log('DONE');
