import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

async function setMode(page, modeName) {
  const combo = page.getByRole('combobox').last();
  await combo.click();
  await sleep(600);
  await page.getByRole('option', { name: modeName }).click();
  await sleep(1500);
}

async function sendAndSettle(page, mark) {
  const composer = page.getByRole('textbox').last();
  await composer.fill(mark);
  await composer.press('Enter');
  // 轮询到 token 变化或审批卡出现，最多 40s
  for (let i = 0; i < 20; i++) {
    await sleep(2000);
    const s = await snapshot(page);
    const hasCard = s.includes('仅允许一次') || s.includes('批准');
    const m = s.match(/Token in (\d+) · out (\d+)/);
    if (hasCard) return { card: true, snap: s };
    if (m) {
      const total = Number(m[1]) + Number(m[2]);
      if (total > 300) return { card: false, snap: s };
    }
  }
  return { card: false, snap: await snapshot(page) };
}

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);
  await page.getByRole('listitem', { name: 'hello' }).first().click();
  await sleep(1200);

  // 切 Allow·放行
  await setMode(page, 'Allow · 放行');
  const s0 = await snapshot(page);
  console.log('切放行回执: ' + s0.includes('权限模式已切换：放行'));
  await shot(page, 'a16_switched_allow');

  // 采样 1
  const r1 = await sendAndSettle(page, 'perm');
  console.log('=== Allow 采样1: 审批卡=' + r1.card + ' ===');
  const i1 = r1.snap.indexOf('07:');
  console.log(r1.snap.slice(r1.snap.indexOf('log "会话对话"'), r1.snap.indexOf('log "会话对话"') + 2600));
  await shot(page, 'a17_allow_perm_sample1');
}, 's06a_allow_b1_1');
console.log('DONE');
