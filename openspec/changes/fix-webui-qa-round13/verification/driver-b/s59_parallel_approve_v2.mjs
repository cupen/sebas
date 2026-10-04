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
  // 聚焦 drip（等待中的会话）
  const rows = page.getByRole('listitem', { name: /.+/ });
  const n = await rows.count();
  for (let i = 0; i < n; i++) {
    const row = rows.nth(i);
    const nm = (await row.textContent().then(t => (t || '').trim().slice(0, 24))) || '';
    if (/work/.test(nm)) continue;
    await row.click();
    await sleep(1000);
    if (/等待你的审批/.test(await snapshot(page))) { console.log('focused waiting session row[' + i + ']'); break; }
  }
  await sleep(1000);
  const once = page.getByRole('button', { name: '仅允许一次' });
  console.log('count=' + (await once.count()));
  for (let i = 0; i < (await once.count()); i++) {
    const b = once.nth(i);
    console.log(`[${i}] visible=${await b.isVisible().catch(e => 'err')} enabled=${await b.isEnabled().catch(e => 'err')}`);
  }
  const box = await once.first().boundingBox().catch(() => null);
  console.log('box: ' + JSON.stringify(box));
  if (box) {
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    console.log('clicked first card via mouse');
    await sleep(1500);
  }
  // 第二张卡
  const once2 = page.getByRole('button', { name: '仅允许一次' });
  if (await once2.isVisible().catch(() => false)) {
    const box2 = await once2.first().boundingBox();
    if (box2) {
      await page.mouse.click(box2.x + box2.width / 2, box2.y + box2.height / 2);
      console.log('clicked second card via mouse');
    }
  }
  await sleep(4000);
  await shot(page, 'b64_parallel_approved');
  const s = await snapshot(page);
  console.log('=== after approvals ===');
  console.log(s.split('\n').filter(l => /过程|paragraph|已执行|已拒绝|finished|等待/.test(l)).slice(0, 20).join('\n'));
}, 's59_parallel_approve_v2');
