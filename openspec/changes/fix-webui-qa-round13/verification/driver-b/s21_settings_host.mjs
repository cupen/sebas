import { withPage, shot, BASE, sleep } from './helper.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await page.waitForLoadState('domcontentloaded');
  await sleep(800);
  await page.getByRole('button', { name: '打开设置' }).click();
  await sleep(1000);
  await page.getByText('模型', { exact: true }).first().click();
  await sleep(800);
  const chain = await page.evaluate(() => {
    // 深搜 shadow DOM 找含「管理模型」的元素，回溯宿主链
    const found = [];
    const walk = (root, hostChain) => {
      for (const el of root.querySelectorAll('*')) {
        const direct = Array.from(el.childNodes).some(n => n.nodeType === 3 && n.textContent.includes('管理模型'));
        if (direct || (el.shadowRoot && el.textContent.includes('管理模型') && !el.shadowRoot.querySelector('*'))) {
          found.push({ tag: el.tagName, chain: hostChain.join('>') });
        }
        if (el.shadowRoot) walk(el.shadowRoot, [...hostChain, el.tagName]);
      }
    };
    walk(document, []);
    return found.slice(0, 10);
  });
  console.log(JSON.stringify(chain, null, 1));
  // 也可直接看顶层元素
  const tops = await page.evaluate(() => Array.from(document.body.children).map(e => e.tagName + '.' + String(e.className).slice(0, 30)));
  console.log('body children: ' + JSON.stringify(tops));
}, 's21_settings_host');
