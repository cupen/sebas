import { withPage, shot, snapshot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  // fake 行的星标按钮（设为默认）
  const row = page.locator('wa-dialog').locator('text=fake').first();
  const starBtns = page.locator('wa-dialog').getByRole('button');
  const n = await starBtns.count();
  console.log('buttons in settings dialog: ' + n);
  for (let i = 0; i < n; i++) {
    const b = starBtns.nth(i);
    const label = (await b.getAttribute('aria-label')) || (await b.getAttribute('title')) || (await b.textContent().then(t => t.trim().slice(0, 20)));
    const vis = await b.isVisible();
    console.log(`btn[${i}] vis=${vis} label=${JSON.stringify(label)}`);
  }
}, 's20_star_probe');
