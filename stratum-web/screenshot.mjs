import { chromium } from '/Users/wanyaozhong/.npm/_npx/e41f203b7505f1fb/node_modules/playwright-core/index.mjs';
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
await page.goto('http://localhost:5173/longzhong', { waitUntil: 'networkidle' });
await page.waitForTimeout(2000);
await page.screenshot({ path: '/tmp/longzhong-1920.png', fullPage: false });

const historyBtn = await page.locator('[data-slot="navbar-left-slot"] button').first();
const btnBox = await historyBtn.boundingBox().catch(() => null);
console.log('history button box:', btnBox);

if (btnBox) {
  await historyBtn.click();
  await page.waitForTimeout(800);
  await page.screenshot({ path: '/tmp/longzhong-history-1920.png', fullPage: false });
}
await browser.close();
console.log('screenshots saved');
