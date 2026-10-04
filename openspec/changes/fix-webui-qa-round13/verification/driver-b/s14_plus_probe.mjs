import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  await page.getByText('新建（自定义）').click();
  await sleep(800);
  const dlg = page.locator('wa-dialog.provider-editor');
  await dlg.locator('wa-input[label="名称"] input').fill('fake');
  await dlg.locator('wa-select[label="协议"]').click();
  await sleep(400);
  await page.locator('wa-option[value="anthropic"]').click();
  await sleep(400);
  await dlg.locator('wa-input[label="API key"] input').fill('sk-sandbox-dummy');

  const addRow = dlg.locator('.model-add, [class*="add"]').first();
  await addRow.click();
  await sleep(500);
  await dlg.locator('input[placeholder="模型 id"]').first().fill('test');
  await sleep(300);
  // 现在找「＋」按钮：dump 模型区域的可点击元素
  const clickables = await dlg.evaluateAll((els) => {
    const out = [];
    const root = els[0]?.closest('body') || document;
    const walk = (n) => {
      for (const el of n.querySelectorAll('*')) {
        const cls = (el.className && String(el.className)) || '';
        const txt = (el.textContent || '').trim();
        if (/plus|add|\+|＋/.test(cls + ' ' + txt) && el.getBoundingClientRect().width > 0) {
          out.push({ tag: el.tagName, cls: cls.slice(0, 80), txt: txt.slice(0, 30), aria: el.getAttribute('aria-label') || el.getAttribute('title') || '' });
        }
        if (el.shadowRoot) {
          for (const s of el.shadowRoot.querySelectorAll('*')) {
            const c2 = (s.className && String(s.className)) || '';
            const t2 = (s.textContent || '').trim();
            if (/plus|add|\+|＋/.test(c2 + ' ' + t2) && s.getBoundingClientRect().width > 0) {
              out.push({ tag: s.tagName + '(shadow of ' + el.tagName + ')', cls: c2.slice(0, 80), txt: t2.slice(0, 30), aria: s.getAttribute('aria-label') || s.getAttribute('title') || '' });
            }
          }
        }
      }
    };
    walk(document);
    return out.slice(0, 30);
  });
  console.log(JSON.stringify(clickables, null, 1));
}, 's14_plus_probe');
