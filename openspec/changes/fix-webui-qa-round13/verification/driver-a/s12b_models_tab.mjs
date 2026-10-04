import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function openSettings(page, tab) {
  await page.goto(BASE + '/');
  await sleep(1200);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  if (tab) {
    await page.getByText(tab, { exact: true }).first().click();
    await sleep(1200);
  }
}

await withPage(async (page) => {
  await openSettings(page, '模型');
  console.log('--- 模型 tab ---');
  const snap = await snapshot(page);
  const di = snap.indexOf('dialog');
  console.log(snap.slice(di >= 0 ? di : 0, (di >= 0 ? di : 0) + 3000));
  await shot(page, 'a34_settings_models_tab');
}, 's12b_models_tab');
console.log('DONE');
