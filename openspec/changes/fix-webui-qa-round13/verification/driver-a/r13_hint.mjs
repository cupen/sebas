import { chromium } from 'file:///D:/workbench/repos-ai/sebas/tests/testsuite-webui/node_modules/@playwright/test/index.mjs';
import path from 'node:path';
const SHOTS = 'D:/workbench/repos-ai/sebas/openspec/changes/fix-webui-qa-round13/verification/shots';
const ctx = await chromium.launchPersistentContext('D:/workbench/repos-ai/sebas/target/qa-r13-hint-profile', {
  headless: true, viewport: { width: 1440, height: 900 }, locale: 'zh-CN',
});
const page = ctx.pages()[0] || (await ctx.newPage());
page.setDefaultTimeout(8000);
await page.goto('http://127.0.0.1:9878/');
await page.waitForTimeout(2500);
// 无项目时先随便注册一个目录才能建会话？对话框入口按钮可能不存在——先拍快照
const s = await page.locator('body').ariaSnapshot();
console.log(s.slice(0, 900));
// 若有「添加项目」：注册 hint home 下的 work
const addBtn = page.getByRole('button', { name: '添加项目' });
if (await addBtn.count()) {
  await addBtn.click();
  await page.waitForTimeout(800);
  await page.getByRole('textbox').last().fill('D:/workbench/repos-ai/sebas/target/qa-r13-hint/work');
  await page.waitForTimeout(400);
  await page.getByRole('button', { name: /注册|添加|确认/ }).last().click();
  await page.waitForTimeout(1200);
}
await page.keyboard.press('Escape'); await page.waitForTimeout(700);
  const newBtn = page.getByRole('button', { name: /新建会话/ });
if (await newBtn.count()) {
  await newBtn.first().click();
  await page.waitForTimeout(900);
  const agentCombo = page.getByRole('combobox').first();
  await agentCombo.click();
  await page.waitForTimeout(500);
  await page.waitForTimeout(300); // 不点禁用项，直接截展开态
  await page.waitForTimeout(700);
  await page.screenshot({ path: path.join(SHOTS, 'r13_native_hint.png') });
  const s2 = await page.locator('body').ariaSnapshot();
  const di = s2.indexOf('dialog');
  console.log('--- dialog ---');
  console.log(s2.slice(Math.max(0, di), di + 1000));
}
await ctx.close();
