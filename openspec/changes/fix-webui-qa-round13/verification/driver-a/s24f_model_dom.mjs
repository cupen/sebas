import { withPage, shot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(2500);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1200);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(1000);
  await page.getByRole('button', { name: '＋ 新建（自定义）' }).click();
  await sleep(800);

  // 找「模型」标题后面的 add 行结构（只读 DOM 探查）
  const info = await page.evaluate(() => {
    const out = [];
    const walk = (el, depth) => {
      if (!el || depth > 22 || out.length > 40) return;
      for (const c of el.children || []) {
        const tag = c.tagName;
        const cls = (c.className && typeof c.className === 'string') ? c.className.slice(0, 60) : '';
        const txt = (c.textContent || '').trim().slice(0, 30);
        const slot = c.getAttribute && c.getAttribute('slot');
        if (slot === 'models' || tag.includes('MODEL') || (cls && cls.includes('model'))) {
          out.push(depth + ': ' + tag + ' | cls=' + cls + ' | slot=' + slot + ' | txt=' + txt);
        }
        walk(c, depth + 1);
      }
    };
    walk(document.body, 0);
    return out;
  });
  console.log(info.join('\n') || '(无 model 关键字节点)');

  // 全 DOM 找 slot=models 或 data-testid 带 model 的
  const info2 = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll('[data-testid], [slot]').forEach(e => {
      const t = e.getAttribute('data-testid') || '';
      const s = e.getAttribute('slot') || '';
      if (t.includes('model') || s.includes('model') || t.includes('provider')) {
        out.push(e.tagName + '|testid=' + t + '|slot=' + s + '|txt=' + (e.textContent || '').trim().slice(0, 40));
      }
    });
    return out;
  });
  console.log('--- testid/slot ---');
  console.log(info2.join('\n') || '(无)');
}, 's24f_dom');
console.log('DONE');
