import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.keyboard.press('Escape');
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1500);
  await page.getByRole('button', { name: 'Agent', exact: true }).click();
  await sleep(1200);
  await shot(page, 'b21_settings_agents');
  const app = page.locator('sebas-app');
  const txt = await page.evaluate(() => {
    const m = document.querySelector('sebas-settings-modal');
    return m ? m.textContent : 'no modal';
  });
  console.log(txt.slice(0, 2000));
}, 's25_settings_agents');
