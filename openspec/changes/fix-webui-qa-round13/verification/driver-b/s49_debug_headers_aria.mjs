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
  const unnamed = page.getByRole('listitem', { name: '未命名会话', exact: true });
  const n = await unnamed.count();
  for (let i = 0; i < n; i++) {
    await unnamed.nth(i).click();
    await sleep(1000);
    const s = await snapshot(page);
    const m = s.split('\n').filter(l => /🔒|text:.*local 逐次询问|未聚焦/.test(l)).join(' ;; ');
    console.log(`[${i}] ` + m.slice(0, 160));
  }
}, 's49_debug_headers_aria');
