import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 切回 Ask
  const combo = page.getByRole('combobox').last();
  await combo.click();
  await sleep(600);
  await page.getByRole('option', { name: 'Ask · 逐次询问' }).click();
  await sleep(1200);

  // 发 parallel
  const composer = page.getByRole('textbox').last();
  await composer.fill('parallel');
  await composer.press('Enter');
  // 等审批卡
  await page.getByText('仅允许一次').first().waitFor({ state: 'visible', timeout: 30000 }).catch(() => {});
  await sleep(1000);
  const s1 = await snapshot(page);
  const i = s1.indexOf('log "会话对话"');
  console.log('--- parallel 卡面 ---');
  console.log(s1.slice(i, i + 3000));
  await shot(page, 'a19_parallel_two_cards');
}, 's07a_parallel');
console.log('DONE');
