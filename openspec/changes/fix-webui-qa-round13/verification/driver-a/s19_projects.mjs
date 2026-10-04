import { withPage, shot, snapshot, sleep, BASE } from './helper-a.mjs';

await withPage(async (page) => {
  await page.goto(BASE + '/');
  await sleep(1500);

  // 注册第二个项目 downloads
  await page.getByRole('button', { name: '添加项目' }).click();
  await sleep(1000);
  await page.getByRole('treeitem', { name: 'downloads' }).click();
  await sleep(2500);
  await page.getByRole('button', { name: '添加项目' }).last().click();
  await sleep(1500);
  const s1 = await snapshot(page);
  console.log('downloads 注册: ' + (s1.includes('downloads local') ? 'OK' : '未见'));
  await shot(page, 'a71_two_projects');

  // 项目操作菜单（downloads）
  const dlMenu = page.getByRole('button', { name: '项目操作：downloads' });
  if (await dlMenu.count() > 0) {
    await dlMenu.click();
    await sleep(800);
    console.log('--- downloads 项目菜单 ---');
    const s2 = await snapshot(page);
    const mi = s2.indexOf('menu');
    console.log(s2.slice(mi >= 0 ? mi : 0, (mi >= 0 ? mi : 0) + 800));
    await shot(page, 'a72_project_menu');
    await page.keyboard.press('Escape');
    await sleep(500);
  }

  // 项目切换器行为：点 work 行 → 主区切到 work；点 downloads 行 → 切到 downloads
  await page.getByText('work local').first().click();
  await sleep(800);
  const sw1 = await snapshot(page);
  console.log('切 work 后主区: ' + (sw1.includes('work local 未聚焦') || sw1.includes('text: work local') ? 'work' : '?'));
  await page.getByText('downloads local').first().click();
  await sleep(800);
  const sw2 = await snapshot(page);
  console.log('切 downloads 后主区含 downloads: ' + sw2.includes('downloads'));
  await shot(page, 'a73_project_switch_downloads');
}, 's19_projects');
console.log('DONE');
