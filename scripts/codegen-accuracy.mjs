// Measures figma_to_code accuracy: renders html-annotated output in headless Chrome
// and compares every layer's box and styles with the get_design_context spec.
//   node scripts/codegen-accuracy.mjs [context.json ...] [--min <percent>] [--all]
// Default context: tests/fixtures/design-context-checkout.json. Needs Chrome and
// network access for the Tailwind play CDN and Google Fonts.
import { execFileSync, spawn } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const args = process.argv.slice(2);
const minIndex = args.indexOf('--min');
const min = minIndex >= 0 ? Number(args.splice(minIndex, 2)[1]) : null;
const showAll = args.includes('--all') && args.splice(args.indexOf('--all'), 1);
const files = args.length ? args : ['tests/fixtures/design-context-checkout.json'];
const binary = process.env.FIGMA_TEST_BINARY || 'target/debug/figma-rust-mcp';
const chromePath = process.env.CHROME_PATH || {
  darwin: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  win32: 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
}[process.platform] || 'google-chrome';

const px = v => (typeof v === 'number' ? v : parseFloat(v));
const box = v => { const p = String(v ?? '').trim().split(/\s+/).map(px); return [p[0], p[1] ?? p[0], p[2] ?? p[0], p[3] ?? p[1] ?? p[0]]; };
const rgb = hex => { const h = hex.replace('#', ''); return `rgb(${parseInt(h.slice(0, 2), 16)}, ${parseInt(h.slice(2, 4), 16)}, ${parseInt(h.slice(4, 6), 16)})`; };
const WEIGHTS = { thin: 100, extralight: 200, light: 300, regular: 400, normal: 400, medium: 500, semibold: 600, bold: 700, extrabold: 800, black: 900 };

function walk(node, out = []) {
  out.push(node);
  for (const child of node.children || []) walk(child, out);
  return out;
}

// Tokens resolve through the layer's own solid paint, so colors compare as rendered.
function tokenCss(layers) {
  const vars = new Map();
  for (const n of layers) {
    const color = n.paintData?.find(p => p.type === 'SOLID' && p.color)?.color;
    for (const ref of [n.fill, n.text?.color]) {
      const name = /^var\((--[^)]+)\)$/.exec(ref || '')?.[1];
      if (name && color && /^#[0-9a-f]{6}$/i.test(color)) vars.set(name, color);
    }
  }
  return `:root{${[...vars].map(([k, v]) => `${k}:${v}`).join(';')}}`;
}

function resolveColor(value, layer) {
  if (!value) return null;
  if (value.startsWith('var(')) value = layer.paintData?.find(p => p.type === 'SOLID')?.color;
  return value && /^#[0-9a-f]{6}$/i.test(value) ? rgb(value) : null;
}

// One entry per measurable property: [group, expected, actual, tolerance].
function checks(layer, got) {
  const out = [];
  const num = (group, expected, actual, tol = 1) => {
    if (Number.isFinite(expected)) out.push([group, expected, actual, tol]);
  };
  num('width', layer.size?.width, got.width);
  num('height', layer.size?.height, got.height);
  const layout = layer.layout;
  if (layout?.padding) box(layout.padding).forEach((v, i) => num('padding', v, got.padding[i], 0.5));
  if (layout?.display === 'flex' && layout.gap != null) {
    num('gap', px(layout.gap), layout.flexDirection === 'column' ? got.rowGap : got.columnGap, 0.5);
  }
  // Browsers clamp radii to half the shorter side (rounded-full is 9999px); compare what renders.
  const clamp = v => Math.min(v, Math.min(got.width, got.height) / 2);
  if (layer.borderRadius) box(layer.borderRadius).forEach((v, i) => num('radius', clamp(v), clamp(got.radius[i]), 0.5));
  const text = layer.text;
  if (layer.type === 'TEXT' && text) {
    num('fontSize', text.fontSize, got.fontSize, 0.1);
    num('fontWeight', text.fontWeightNumeric ?? WEIGHTS[String(text.fontWeight).toLowerCase().replace(/\s/g, '')], got.fontWeight, 0);
    if (String(text.lineHeight).endsWith('%')) num('lineHeight', text.fontSize * px(text.lineHeight) / 100, got.lineHeight, 0.5);
    else if (text.lineHeight != null && text.lineHeight !== 'auto') num('lineHeight', px(text.lineHeight), got.lineHeight, 0.5);
    const color = resolveColor(text.color, layer);
    if (color) out.push(['color', color, got.color, 0]);
  } else if (layer.backgroundCss?.property === 'background-color') {
    const bg = resolveColor(layer.backgroundCss.value, layer);
    if (bg) out.push(['background', bg, got.background, 0]);
  }
  return out;
}

async function launchChrome() {
  const profile = mkdtempSync(join(tmpdir(), 'codegen-accuracy-'));
  const chrome = spawn(chromePath, ['--headless=new', '--remote-debugging-port=0', `--user-data-dir=${profile}`, '--hide-scrollbars', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
  const port = await new Promise((resolve, reject) => {
    let log = '';
    chrome.stderr.on('data', b => { log += b; const m = /DevTools listening on ws:\/\/[^:]+:(\d+)/.exec(log); if (m) resolve(m[1]); });
    chrome.on('exit', code => reject(new Error(`Chrome exited (${code}): ${log.slice(-500)}`)));
  });
  return { chrome, port };
}

async function cdpPage(port) {
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })).json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const pending = new Map();
  const events = [];
  ws.onmessage = e => {
    const msg = JSON.parse(e.data);
    if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); } else events.push(msg);
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const msgId = ++id;
    pending.set(msgId, msg => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
    ws.send(JSON.stringify({ id: msgId, method, params }));
  });
  return { send, events, close: () => ws.close() };
}

const MEASURE = `(() => [...document.querySelectorAll('[data-node-id]')].map(el => {
  const r = el.getBoundingClientRect(), s = getComputedStyle(el), n = v => parseFloat(v) || 0;
  return { id: el.dataset.nodeId, width: r.width, height: r.height,
    padding: [n(s.paddingTop), n(s.paddingRight), n(s.paddingBottom), n(s.paddingLeft)],
    rowGap: n(s.rowGap), columnGap: n(s.columnGap),
    radius: [n(s.borderTopLeftRadius), n(s.borderTopRightRadius), n(s.borderBottomRightRadius), n(s.borderBottomLeftRadius)],
    fontSize: n(s.fontSize), fontWeight: n(s.fontWeight), lineHeight: s.lineHeight === 'normal' ? NaN : n(s.lineHeight),
    color: s.color, background: s.backgroundColor };
}))()`;

async function measure(page, file) {
  const raw = JSON.parse(readFileSync(file, 'utf8'));
  const root = raw.context || raw;
  const layers = walk(root);
  const html = execFileSync(binary, ['--codegen', 'html-annotated', file], { encoding: 'utf8' });
  const families = [...new Set(layers.map(n => n.text?.fontFamily).filter(Boolean))];
  const fonts = families.map(f => `family=${encodeURIComponent(f)}:ital,wght@0,100;0,300;0,400;0,700;0,900;1,400`).join('&');
  const doc = `<!doctype html><html><head><meta charset="utf-8">
<script src="https://cdn.tailwindcss.com/3.4.17"></script>
${fonts ? `<link rel="stylesheet" href="https://fonts.googleapis.com/css2?${fonts}&display=block">` : ''}
<style>${tokenCss(layers)} body{margin:0}</style></head><body>${html}</body></html>`;
  const path = join(mkdtempSync(join(tmpdir(), 'codegen-page-')), 'index.html');
  writeFileSync(path, doc);
  const width = Math.ceil(root.size?.width || 1440);
  await page.send('Emulation.setDeviceMetricsOverride', { width, height: Math.ceil(root.size?.height || 900), deviceScaleFactor: 1, mobile: false });
  await page.send('Page.navigate', { url: `file://${path}` });
  await page.send('Runtime.evaluate', { expression: 'new Promise(r => (document.readyState === "complete" ? r() : addEventListener("load", r))).then(() => document.fonts.ready).then(() => new Promise(r => setTimeout(r, 300)))', awaitPromise: true });
  const { result } = await page.send('Runtime.evaluate', { expression: MEASURE, returnByValue: true });
  const rendered = new Map(result.value.map(m => [m.id, m]));
  const byGroup = new Map();
  const misses = [];
  let missing = 0;
  for (const layer of layers) {
    const got = rendered.get(layer.id);
    if (!got) { if (layer.id && !layer.isMask) missing++; continue; }
    for (const [group, expected, actual, tol] of checks(layer, got)) {
      const ok = typeof expected === 'number' ? Math.abs(expected - actual) <= tol : expected === actual;
      const g = byGroup.get(group) || { pass: 0, total: 0 };
      g.total++; if (ok) g.pass++;
      byGroup.set(group, g);
      if (!ok) misses.push({ group, layer: `${layer.name} (${layer.id})`, expected, actual: typeof actual === 'number' ? Math.round(actual * 100) / 100 : actual });
    }
  }
  return { byGroup, misses, missing, layers: layers.length };
}

const { chrome, port } = await launchChrome();
let worst = 100;
try {
  const page = await cdpPage(port);
  await page.send('Page.enable');
  for (const file of files) {
    const { byGroup, misses, missing, layers } = await measure(page, file);
    const pass = [...byGroup.values()].reduce((a, g) => a + g.pass, 0);
    const total = [...byGroup.values()].reduce((a, g) => a + g.total, 0);
    const pct = total ? (pass * 100) / total : 0;
    worst = Math.min(worst, pct);
    console.log(`\n${file}: ${pct.toFixed(1)}% of ${total} checks match (${layers} layers${missing ? `, ${missing} not rendered` : ''})`);
    for (const [group, g] of [...byGroup].sort()) console.log(`  ${group.padEnd(11)} ${String(g.pass).padStart(4)}/${String(g.total).padEnd(4)} ${((g.pass * 100) / g.total).toFixed(1)}%`);
    for (const m of showAll ? misses : misses.slice(0, 12)) console.log(`  ✗ ${m.group} ${m.layer}: expected ${m.expected}, got ${m.actual}`);
    if (!showAll && misses.length > 12) console.log(`  … ${misses.length - 12} more`);
  }
  page.close();
} finally {
  chrome.kill();
}
if (min != null && worst < min) {
  console.error(`\nAccuracy ${worst.toFixed(1)}% is below --min ${min}%`);
  process.exit(1);
}
