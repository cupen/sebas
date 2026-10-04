import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  const modal = page.locator('sebas-settings-modal');
  const btns = modal.getByRole('button');
  const n = await btns.count();
  console.log('buttons in settings modal: ' + n);
  for (let i = 0; i < n; i++) {
    const b = btns.nth(i);
    if (!(await b.isVisible())) continue;
    const label = (await b.getAttribute('aria-label').catch(() => '')) || (await b.getAttribute('title').catch(() => '')) || (await b.textContent().then(t => t.trim().slice(0, 24)));
    console.log(`btn[${i}] label=${JSON.stringify(label)}`);
  }
}, 's22_star_probe_v2');
