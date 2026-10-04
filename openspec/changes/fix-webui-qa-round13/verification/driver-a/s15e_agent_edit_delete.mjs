import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, 'Agent');
  // 编辑 claude-think（第二个 ✎）
  const editBtns = page.getByRole('button', { name: '✎' });
  console.log('✎ 数: ' + await editBtns.count());
  await editBtns.last().click();
  await sleep(900);
  // 改显示名
  const display = page.getByTestId('agent-form-display').locator('input');
  console.log('编辑表单显示名当前: ' + JSON.stringify(await display.inputValue()));
  await display.fill('claude-think（改名测试）');
  await page.getByRole('button', { name: '保存' }).click();
  await sleep(1200);
  const s1 = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('改名后列表含新名: ' + s1.includes('claude-think（改名测试）'));
  await shot(page, 'a55_agent_renamed');

  // 删除 claude-think（第二个 🗑）
  const delBtns = page.getByRole('button', { name: '🗑' });
  await delBtns.last().click();
  await sleep(800);
  console.log('--- 删除 agent 确认弹窗（目验截图）---');
  await shot(page, 'a56_agent_delete_confirm');
  const okBtn = page.getByRole('button', { name: /删除|确认|确定/ }).last();
  await okBtn.click();
  await sleep(1200);
  const s2 = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('删除后列表仍含 claude-think: ' + s2.includes('claude-think'));
  await shot(page, 'a57_agent_deleted');

  // 新建会话下拉不再出现
  await page.keyboard.press('Escape');
  await sleep(600);
  await page.getByRole('button', { name: '在 work 中新建会话' }).click();
  await sleep(900);
  const combo = page.getByRole('combobox').first();
  await combo.click();
  await sleep(600);
  const s3 = await snapshot(page);
  const li = s3.indexOf('listbox');
  console.log('--- 删除后 Agent 下拉 ---');
  console.log(s3.slice(li, li + 400));
  await shot(page, 'a58_agent_gone_dropdown');
  await page.keyboard.press('Escape');
}, 's15e_edit_delete');
console.log('DONE');
