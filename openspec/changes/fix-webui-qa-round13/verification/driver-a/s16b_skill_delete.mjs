import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettingsTab(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText(tab, { exact: true }).first().click();
  await sleep(1000);
}

await withPage(async (page) => {
  await openSettingsTab(page, '技能');
  // 刷新
  await page.getByRole('button', { name: '刷新' }).click();
  await sleep(1500);
  let s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('刷新后仍在: ' + (s.includes('skill-alpha') && s.includes('skill-beta')));
  await shot(page, 'a60_skills_refreshed');

  // 删除 skill-alpha
  await page.getByRole('button', { name: '🗑' }).first().click();
  await sleep(800);
  await shot(page, 'a61_skill_delete_confirm');
  // 确认
  const okBtn = page.getByRole('button', { name: /^删除|确认|确定$/ }).last();
  console.log('确认按钮: ' + JSON.stringify(await okBtn.textContent().catch(() => null)));
  await okBtn.click();
  await sleep(1500);
  s = await page.locator('[role="dialog"]').first().ariaSnapshot();
  console.log('删除后列表: ' + (s.includes('skill-alpha') ? 'alpha 仍在(异常)' : 'alpha 已消失') + '；beta 在: ' + s.includes('skill-beta'));
  await shot(page, 'a62_skill_deleted');
}, 's16b_skill_delete');
console.log('DONE');
