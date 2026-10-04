import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(1000);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1500);
  await page.getByRole('button', { name: '模型', exact: true }).click();
  await sleep(1200);
  await shot(page, 'b19a_before_star');
  const modal = page.locator('sebas-settings-modal');
  const star = modal.getByRole('button', { name: '设为新建会话的默认' });
  console.log('star count: ' + (await star.count()) + ' visible: ' + (await star.isVisible().catch(e => 'err')));
  const box = await star.boundingBox().catch(() => null);
  console.log('box: ' + JSON.stringify(box));
  if (box) {
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    console.log('clicked via mouse');
  }
  await sleep(1200);
  await shot(page, 'b19_default_set');
  const txt = await modal.innerText();
  console.log('=== modal text (head) ===');
  console.log(txt.slice(0, 600));
}, 's23b_set_default_v2');
