import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 仍在 Allow 档（上一脚本遗留状态），直接发 perm —— 采样2
  const comboTxt = await page.getByRole('combobox').last().textContent().catch(() => null);
  console.log('当前 combobox: ' + JSON.stringify(comboTxt));
  const composer = page.getByRole('textbox').last();
  await composer.fill('perm');
  await composer.press('Enter');
  for (let i = 0; i < 20; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    const m = s.match(/Token in (\d+) · out (\d+)/);
    if (m && Number(m[1]) >= 500) break;
  }
  const s2 = await snapshot(page);
  const tail = s2.slice(s2.lastIndexOf('paragraph: perm'), s2.lastIndexOf('paragraph: perm') + 700);
  console.log('=== Allow 采样2 尾部 ===');
  console.log(tail);
  console.log('审批卡出现: ' + s2.includes('仅允许一次'));
  await shot(page, 'a18_allow_perm_sample2');
}, 's06b_allow_b1_2');
console.log('DONE');
