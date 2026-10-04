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
  await page.getByRole('listitem', { name: 'line1 line2', exact: true }).click();
  await sleep(1200);
  const head = await page.locator('main').innerText();
  console.log('=== header ===');
  console.log(head.split('\n').slice(0, 6).join(' | '));
  console.log('=== before expand ===');
  const s1 = await snapshot(page);
  console.log(s1.split('\n').filter(l => /过程|thought|answer|paragraph/.test(l)).join('\n'));
  await shot(page, 'b47_thinking_collapsed');
  const chip = page.getByRole('button', { name: /过程 thinking/ }).first();
  await chip.click();
  await sleep(700);
  console.log('=== after expand ===');
  const s2 = await snapshot(page);
  console.log(s2.split('\n').filter(l => /过程|thought|answer|paragraph|展开|收起/.test(l)).join('\n'));
  await shot(page, 'b48_thinking_expanded');
}, 's46_thinking_expand_v2');
