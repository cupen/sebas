import { withPage, snapshot, sleep, BASE } from './helper-a.mjs';
await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  console.log('URL: ' + page.url());
  const s = await snapshot(page);
  console.log(s.slice(0, 700));
}, 'r13_diag');
