import { chromium } from '@playwright/test';
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';

const REPO = fileURLToPath(new URL('../../../', import.meta.url));
const faces = [
  { weight: '400', file: 'NotoSansJP-Regular.ttf' },
  { weight: '700', file: 'NotoSansJP-Bold.ttf' },
].map((f) => ({ ...f, b64: fs.readFileSync(`${REPO}/fonts/Noto_Sans_JP/static/${f.file}`).toString('base64') }));

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });
await page.setContent('<html><body style="margin:0"><canvas id="c" width="400" height="120"></canvas></body></html>');

await page.evaluate(async (faces) => {
  const toBuf = (b64) => { const b = atob(b64); const u = new Uint8Array(b.length); for (let i=0;i<b.length;i++) u[i]=b.charCodeAt(i); return u.buffer; };
  for (const f of faces) {
    const face = new FontFace('Noto Sans JP', toBuf(f.b64), { weight: f.weight, style: 'normal' });
    await face.load(); document.fonts.add(face);
  }
  await document.fonts.ready;
}, faces);

await page.evaluate(() => {
  const ctx = document.getElementById('c').getContext('2d');
  ctx.fillStyle = '#ffffff'; ctx.fillRect(0, 0, 400, 120);
  ctx.textBaseline = 'alphabetic';
  ctx.font = "12.00pt 'Noto Sans JP'";
  ctx.fillStyle = '#101820';
  ctx.fillText('横浜市市税条例 Hgあ', 20, 40);
  ctx.fillStyle = '#c00000';
  ctx.fillRect(20, 60, 100, 10);
  ctx.strokeStyle = '#0057b8';
  ctx.lineWidth = 2;
  ctx.beginPath(); ctx.moveTo(20, 90); ctx.lineTo(380, 90); ctx.stroke();
});

const buf = await page.locator('#c').screenshot();
fs.writeFileSync(process.argv[2], buf);
console.error('wrote', process.argv[2]);
await browser.close();
