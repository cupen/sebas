import { withPage, shot, sleep, BASE } from './helper-a.mjs';

async function openSettings(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  if (tab) {
    await page.getByText(tab, { exact: true }).first().click();
    await sleep(1000);
  }
}

await withPage(async (page) => {
  await openSettings(page, '模型');
  // 探容器
  for (const sel of ['[role="dialog"]', 'dialog', '[class*="modal" i]', '[class*="drawer" i]', '[class*="overlay" i]', '[class*="settings" i]']) {
    console.log(sel + ' → ' + await page.locator(sel).count());
  }
  // 全文快照找 模型 段
  const bodySnap = await page.locator('body').ariaSnapshot();
  const mi = bodySnap.indexOf('添加 provider');
  console.log('body 快照含「添加 provider」: ' + (mi >= 0));
  const pi = bodySnap.indexOf('Provider');
  console.log('body 快照含「Provider」: ' + (pi >= 0));
  // 模型 tab 区域内的快照：取「模型」文本之后的部分
  const ti = bodySnap.lastIndexOf('模型');
  console.log('--- body 快照 模型 段 ---');
  console.log(bodySnap.slice(Math.max(0, ti - 200), ti + 2500));
}, 's12c_probe');
console.log('DONE');
