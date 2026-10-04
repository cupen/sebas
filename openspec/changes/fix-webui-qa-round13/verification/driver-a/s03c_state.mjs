import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1800);
  console.log(await snapshot(page));
  await shot(page, 'a07_project_registered');
}, 's03c_state');
console.log('DONE');
