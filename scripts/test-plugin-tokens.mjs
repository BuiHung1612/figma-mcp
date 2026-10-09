import { test } from 'node:test';
import { indexSyncUi } from './helpers/index-sync-ui.mjs';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { spawnSync } from 'node:child_process';
import { configureAgent, detectAgents } from '../bin/agent-config.js';
import { parse as parseJsonc } from 'jsonc-parser';

function runtime(figma = {}) {
  const context = vm.createContext({ figma, handlers: {}, console, setTimeout, clearTimeout, Map, Set, Uint8Array });
  for (const file of ['utils', 'token-helpers', 'read-helpers', 'handlers-read-detail', 'handlers-read', 'handlers-tokens', 'handlers-write-ops', 'node-reader']) {
    vm.runInContext(readFileSync(`plugin-src/${file}.js`, 'utf8'), context, { filename: file });
  }
  return context;
}
const plain = value => JSON.parse(JSON.stringify(value));
const gradient = (transform = [[1, 0, 0], [0, 1, 0]]) => ({
  type: 'GRADIENT_LINEAR', gradientTransform: transform, opacity: .5,
  gradientStops: [{ position: 0, color: { r: 1, g: 0, b: 0, a: .5 } }, { position: 1, color: { r: 0, g: 0, b: 1, a: 1 } }],
});
const shadow = type => ({ type, color: { r: 0, g: 0, b: 0, a: .125 }, offset: { x: -1, y: 3 }, radius: 6, spread: -2, visible: true });

test('compact tree and scan preserve mixed typography instead of using the first run', async () => {
  const mixed = Symbol('mixed');
  const segments = [
    { start: 0, end: 5, characters: 'Hello', fontSize: 15.5, fontName: { family: 'Inter', style: 'Regular' } },
    { start: 5, end: 6, characters: '!', fontSize: 22, fontName: { family: 'Inter', style: 'Semi Bold' } },
  ];
  const node = { id: '1:1', name: 'Greeting', type: 'TEXT', characters: 'Hello!',
    fontSize: mixed, fontName: mixed, visible: true, getStyledTextSegments: () => segments };
  const r = runtime({ root: { id: '0:0' }, currentPage: { id: '0:1' }, getNodeByIdAsync: async () => node });
  vm.runInContext(readFileSync('plugin-src/read-helpers.js', 'utf8'), r);
  for (const detail of ['compact', 'full']) {
    const tokens = { colors: new Set(), fonts: new Set(), sizes: new Set() };
    const tree = plain(r.extractDesignTree(node, 0, 10, detail, true, tokens));
    assert.equal(tree.mixedStyles, true);
    assert.equal(tree.fontSize, undefined);
    assert.equal(tree.fontWeight, undefined);
    assert.equal(tree.fontFamily, 'Inter');
    assert.deepEqual(tree.segments.map(s => [s.start, s.end, s.fontSize, s.fontWeight]),
      [[0, 5, 15.5, 'Regular'], [5, 6, 22, 'Semi Bold']]);
    assert.deepEqual([...tokens.fonts], ['Inter/Regular/15.5px', 'Inter/Semi Bold/22px']);
  }
  const scan = plain(await r.handlers.scan_design({ id: '1:1' }));
  assert.equal(scan.allText[0].fontSize, null);
  assert.equal(scan.allText[0].fontWeight, null);
  assert.equal(scan.allText[0].segments.length, 2);
  assert.deepEqual(scan.allFonts.map(f => f.font).sort(), ['Inter/Regular/15.5px', 'Inter/Semi Bold/22px']);
  segments[1].fontSize = 15.5;
  assert.equal(r.resolveTextStyle(node).fontSize, 15.5);
  assert.equal(r.resolveTextStyle(node).fontWeight, undefined);
});

test('scan_design expands instances by default and flags opaque ones as incomplete', async () => {
  const text = { id: '1:3', name: 'Label', type: 'TEXT', characters: 'Hi', fontSize: 12, fontName: { family: 'Inter', style: 'Regular' }, visible: true };
  const inst = { id: '1:2', name: 'Button', type: 'INSTANCE', visible: true, children: [text], getMainComponentAsync: async () => null };
  const frame = { id: '1:1', name: 'Frame', type: 'FRAME', visible: true, children: [inst] };
  const r = runtime({ root: { id: '0:0' }, currentPage: { id: '0:1' }, getNodeByIdAsync: async () => frame });
  const full = plain(await r.handlers.scan_design({ id: '1:1' }));
  assert.equal(full.totals.textNodes, 1);
  assert.equal(full.complete, true);
  const opaque = plain(await r.handlers.scan_design({ id: '1:1', expandInstances: false }));
  assert.equal(opaque.totals.textNodes, 0);
  assert.equal(opaque.totals.opaqueInstances, 1);
  assert.equal(opaque.complete, false);
});

test('equivalent CSS syntaxes preserve alpha in reads and writes', () => {
  const r = runtime();
  for (const color of ['rgba(255, 0, 0, .5)', 'rgb(100% 0% 0% / 50%)', 'hsl(0 100% 50% / .5)', 'hsla(0, 100%, 50%, .5)']) {
    assert.deepEqual(plain(r.parseColorValue(color)), { r: 1, g: 0, b: 0, a: .5 });
    assert.equal(r.hexToRgbA(color).a, .5);
    assert.equal(r.solidFill(color)[0].opacity, .5);
  }
  assert.equal(r.parseColorValue('#f008').a, 136 / 255);
  assert.equal(r.parseColorValue('transparent').a, 0);
  assert.equal(r.colorToCss({ r: 0, g: 0, b: 0, a: .9999 }), 'rgba(0, 0, 0, 0.9999)');
  assert.throws(() => r.parseColorValue('rgba(garbage)'), /Invalid/);
});

test('gradient projection includes translations, aspect ratio, stop and paint alpha', () => {
  const r = runtime();
  const p = r.serializePaint(gradient());
  assert.deepEqual(plain(p.gradientTransform), [[1, 0, 0], [0, 1, 0]]);
  assert.match(r.paintToCss(p, 200, 100), /^linear-gradient\(90deg, rgba\(255, 0, 0, 0.25\) 0%, rgba\(0, 0, 255, 0.5\) 100%\)$/);
  const diagonal = r.paintToCss(r.serializePaint(gradient([[1, 1, 0], [0, 1, 0]])), 200, 100);
  assert.match(diagonal, /153\.434948/);
  assert.match(diagonal, /50%/);
  const shifted = r.paintToCss(r.serializePaint(gradient([[.5, 0, .25], [0, 1, 0]])), 200, 100);
  assert.match(shifted, /-50%/);
  assert.match(shifted, /150%/);
  assert.throws(() => r.paintToCss({ ...p, type: 'GRADIENT_DIAMOND' }, 100, 100), /SVG/);
});

test('get_css uses the numeric paint, never hex slicing a serialized rgba', async () => {
  const node = { id: '1:2', name: 'alpha', type: 'RECTANGLE', x: 0, y: 0, width: 200, height: 100,
    fills: [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 }, opacity: .25 }], effects: [shadow('DROP_SHADOW'), shadow('INNER_SHADOW')] };
  const r = runtime();
  r.findNodeByIdAsync = async () => node;
  const result = await r.handlers.get_css({ id: node.id });
  assert.match(result.css, /background-color: rgba\(255, 0, 0, 0.25\);/);
  assert.doesNotMatch(result.css, /NaN|0.0625/);
  assert.match(result.css, /inset -1px 3px 6px -2px rgba\(0, 0, 0, 0.125\)/);
  assert.equal(result.detail.effects.length, 2);
});

test('styles retain full paints, effects, units and disabled layers', async () => {
  const r = runtime({
    getLocalPaintStylesAsync: async () => [{ id: 'p', name: 'Gradient', paints: [gradient()] }],
    getLocalTextStylesAsync: async () => [{ id: 't', name: 'Body', fontName: { family: 'Inter', style: 'Regular' }, fontSize: 16, lineHeight: { unit: 'PERCENT', value: 150 }, letterSpacing: { unit: 'PERCENT', value: 2 } }],
    getLocalEffectStylesAsync: async () => [{ id: 's', name: 'Card', effects: [shadow('DROP_SHADOW'), shadow('INNER_SHADOW'), { type: 'LAYER_BLUR', radius: 8, visible: false }] }],
    getLocalGridStylesAsync: async () => [],
  });
  const styles = await r.handlers.get_styles();
  assert.equal(styles.schemaVersion, 2);
  assert.equal(styles.paintStyles[0].paints[0].gradientStops[0].rgba.a, .5);
  assert.equal(styles.effectStyles[0].effects.length, 3);
  assert.equal(styles.textStyles[0].lineHeightUnit, 'PERCENT');
  assert.equal(r.effectsToCss(styles.effectStyles[0].effects).filter, '');
});

test('aliases use mode names across collections and retain false/zero values', async () => {
  const cols = [
    { id: 'semantic', name: 'Semantic', defaultModeId: 's-light', modes: [{ modeId: 's-light', name: 'Light' }, { modeId: 's-dark', name: 'Dark' }], variableIds: ['alias', 'zero'] },
    { id: 'primitive', name: 'Primitive', defaultModeId: 'p-light', modes: [{ modeId: 'p-light', name: 'Light' }, { modeId: 'p-dark', name: 'Dark' }], variableIds: ['base'] },
  ];
  const variables = [
    { id: 'alias', name: 'semantic', variableCollectionId: 'semantic', resolvedType: 'BOOLEAN', valuesByMode: { 's-light': { type: 'VARIABLE_ALIAS', id: 'base' }, 's-dark': { type: 'VARIABLE_ALIAS', id: 'base' } } },
    { id: 'zero', name: 'zero', variableCollectionId: 'semantic', resolvedType: 'FLOAT', valuesByMode: { 's-light': 0, 's-dark': 1 } },
    { id: 'base', name: 'base', variableCollectionId: 'primitive', resolvedType: 'BOOLEAN', valuesByMode: { 'p-light': false, 'p-dark': true } },
  ];
  const r = runtime({ variables: {
    getLocalVariableCollectionsAsync: async () => cols, getLocalVariablesAsync: async () => variables,
    getVariableByIdAsync: async id => variables.find(v => v.id === id),
    getVariableCollectionByIdAsync: async id => cols.find(c => c.id === id),
  } });
  const result = await r.handlers.get_variables();
  const alias = result.collections[0].variables[0];
  assert.equal(alias.values['s-light'].resolvedValue, false);
  assert.equal(alias.values['s-dark'].resolvedValue, true);
  assert.equal(result.resolvedTokens.semantic, false);
  assert.equal(result.resolvedTokens.zero, 0);
  assert.equal(result.collections[0].defaultModeId, 's-light');
  assert.deepEqual(plain(result.diagnostics), []);
  const consumer = { resolvedVariableModes: { semantic: 's-dark', primitive: 'p-light' } };
  variables[0].resolveForConsumer = () => ({ value: false, resolvedType: 'BOOLEAN' });
  assert.equal((await r.resolveVariableValueAsync(variables[0], consumer)).resolvedValue, false);
});

test('export_node validates format and works through batch and aliases', async () => {
  const r = runtime();
  r.handlers.export_svg = async () => ({ svg: '<svg/>' });
  r.handlers.export_image = async params => ({ format: params.format });
  assert.equal((await r.resolveOperationHandler('exportNode')({ format: 'svg' })).svg, '<svg/>');
  assert.equal((await r.handlers.export_node({ format: 'jpeg' })).format, 'JPG');
  const batch = await r.handlers.batch({ operations: [{ operation: 'export_node', params: { format: 'SVG' } }, { operation: 'export_node', params: { format: 'bad' } }] });
  assert.equal(batch.succeeded, 1);
  assert.match(batch.results[1].error, /supports PNG/);
});

test('multiple paints: Figma bottom→top becomes CSS top-first; top-most solid wins', () => {
  const r = runtime();
  const raw = [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 }, opacity: .5 }, { type: 'SOLID', color: { r: 0, g: 0, b: 1 }, opacity: 1 }];
  const css = r.paintsToCss(raw.map(r.serializePaint), 200, 100);
  assert.match(css.value, /^linear-gradient\(#0000ff, #0000ff\)/);
  assert.match(css.value, /linear-gradient\(rgba\(255, 0, 0, 0.5\), rgba\(255, 0, 0, 0.5\)\)$/);
  assert.equal(r.firstSolidHex(raw), '#0000ff');
});

test('UI badge uses release version initially and follows the server on reconnect', () => {
  const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  assert.ok(html.includes(`id="runtime-version">v${pkg.version}</span>`));
  const start = html.indexOf('    function updateRuntimeVersion(');
  const end = html.indexOf('    var runtimeCapabilities', start);
  const badge = {};
  const context = vm.createContext({ document: { getElementById: () => badge } });
  vm.runInContext(html.slice(start, end), context);
  context.updateRuntimeVersion('3.2.7');
  assert.equal(badge.textContent, 'v3.2.7');
  context.updateRuntimeVersion('3.3.0');
  assert.equal(badge.textContent, 'v3.3.0');
  assert.ok(html.includes('updateRuntimeVersion(serverVer)'));
  assert.ok(html.includes('type: "runtime-ready"'));
});

test('sync waits for a matching bridge confirmation and ignores final and silent progress', async () => {
  const ui = indexSyncUi(), messages = [];
  const page = { id: 'page', children: [] };
  const r = runtime({ currentPage: page, root: { name: 'File' },
    ui: { postMessage: message => { messages.push(message); ui.handleIndexMessage(message); } } });
  r.handlers.get_styles = async () => ({});
  r.handlers.get_variables = async () => ({});
  for (let i = 0; i < 2; i++) {
    await r.handlers.index_scan({ deferComponents: true });
    assert.equal(messages.at(-1).stage, 'done');
    assert.equal(ui.reindexBtn.disabled, true);
    assert.match(ui.indexProgressText.textContent, /confirmation/);
    ui.receiveIndexAck({ scanId: 'old', pageId: 'page', success: true });
    assert.equal(ui.reindexBtn.disabled, true);
    const update = messages.findLast(msg => msg.type === 'index-update');
    ui.receiveIndexAck({ ...update, success: true });
    assert.equal(ui.reindexBtn.disabled, false);
    assert.equal(ui.reindexBtn.classList.contains('indexing'), false);
    assert.match(ui.reindexBtn.title, /Last synced/);
    ui.handleIndexMessage({ ...update, type: 'index-progress', stage: 'done' });
    ui.handleIndexMessage({ ...update, type: 'index-progress', stage: 'starting', silent: true });
    assert.equal(ui.reindexBtn.disabled, false);
    ui.fireTimers(1200);
    assert.equal(ui.indexProgressWrap.classList.contains('active'), false);
  }
});

test('sync reports read failures, page cancellation, rejection, missing ack and disconnect', async () => {
  const ui = indexSyncUi();
  const page = { id: 'page', children: [] };
  const r = runtime({ currentPage: page, root: { name: 'File' },
    ui: { postMessage: message => ui.handleIndexMessage(message) } });
  r.handlers.read_nodes = async () => { throw new Error('Cannot read layers'); };
  await assert.rejects(r.handlers.index_scan({}), /Cannot read layers/);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /Cannot read layers/);
  r.handlers.read_nodes = async () => {
    r.figma.currentPage = { id: 'next' };
    return { revision: 0, scope: { id: 'page' }, nodes: [] };
  };
  await r.handlers.index_scan({});
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /Document changed/);
  const start = { type: 'index-start', scanId: 'retry', pageId: 'page' };
  ui.handleIndexMessage(start);
  ui.receiveIndexAck({ ...start, success: false, error: 'Rejected stale revision' });
  assert.match(ui.indexProgressText.textContent, /Rejected stale revision/);
  ui.handleIndexMessage(start);
  ui.handleIndexMessage({ ...start, type: 'index-update' });
  ui.fireTimers(30000);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /No bridge confirmation/);
  ui.ws.readyState = 3;
  ui.handleIndexMessage(start);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /disconnected/);
  assert.equal(ui.reindexBtn.title, 'Sync index');
  ui.ws.readyState = 1;
  ui.parent = { postMessage() {} };
  ui.triggerManualReindex();
  ui.fireTimers(30000);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /Canvas did not start/);
  ui.handleIndexMessage(start);
  ui.receiveIndexAck({ ...start, success: true });
  ui.handleIndexMessage({ ...start, scanId: 'new' });
  ui.fireTimers(1200);
  assert.equal(ui.indexProgressWrap.classList.contains('active'), true);
});

test('build rejects wrong release tags and checked-in bundle matches source', () => {
  const check = spawnSync(process.execPath, ['scripts/build-plugin.js', '--check'], { encoding: 'utf8' });
  assert.equal(check.status, 0, check.stderr);
  const rejected = spawnSync(process.execPath, ['scripts/build-plugin.js'], {
    encoding: 'utf8', env: { ...process.env, GITHUB_REF_TYPE: 'tag', GITHUB_REF_NAME: 'v0.0.0-invalid' },
  });
  assert.notEqual(rejected.status, 0);
  assert.match(rejected.stderr, /does not match build version/);
});

test('setup command prints scriptable choices when no terminal is attached', () => {
  const result = spawnSync(process.execPath, ['bin/figma-rust-mcp.js', '--init'], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /--install-service/);
  assert.match(result.stdout, /--setup-plugin/);
  assert.match(result.stdout, /--service-status/);
});

test('agent setup detects installed clients and preserves existing JSONC and TOML config', () => {
  const home = mkdtempSync(path.join(os.tmpdir(), 'figma-rust-mcp-'));
  try {
    const env = { PATH: '' };
    for (const dir of ['.codex', '.claude', '.antigravity-ide', '.cursor', '.codeium/windsurf', '.copilot', '.config/zed']) {
      mkdirSync(path.join(home, dir), { recursive: true });
    }
    const agents = detectAgents({ home, env, platform: 'linux' });
    assert.deepEqual(agents.map(agent => agent.id), ['codex', 'claude', 'antigravity', 'cursor', 'windsurf', 'vscode', 'zed']);

    for (const id of ['claude', 'antigravity', 'windsurf', 'vscode']) {
      const agent = agents.find(candidate => candidate.id === id);
      configureAgent(agent);
      const config = parseJsonc(readFileSync(agent.configPath, 'utf8')).mcpServers['figma-rust-mcp'];
      assert.ok(config);
      if (id === 'antigravity') assert.equal(config.serverUrl, 'http://127.0.0.1:41730/sse');
      else assert.equal(config.url, 'http://127.0.0.1:41730/mcp');
    }

    const cursor = agents.find(agent => agent.id === 'cursor');
    writeFileSync(cursor.configPath, '{\n  // keep this note\n  "mcpServers": { "other": { "url": "http://example.test" } }\n}\n');
    assert.equal(configureAgent(cursor), true);
    const cursorText = readFileSync(cursor.configPath, 'utf8');
    const cursorConfig = parseJsonc(cursorText);
    assert.match(cursorText, /keep this note/);
    assert.equal(cursorConfig.mcpServers.other.url, 'http://example.test');
    assert.equal(cursorConfig.mcpServers['figma-rust-mcp'].url, 'http://127.0.0.1:41730/mcp');
    assert.equal(configureAgent(cursor), false);

    const zed = agents.find(agent => agent.id === 'zed');
    writeFileSync(zed.configPath, '{\n  "theme": "One Dark",\n}\n');
    configureAgent(zed);
    assert.equal(parseJsonc(readFileSync(zed.configPath, 'utf8')).theme, 'One Dark');
    assert.equal(parseJsonc(readFileSync(zed.configPath, 'utf8')).context_servers['figma-rust-mcp'].url, 'http://127.0.0.1:41730/mcp');

    const codex = agents.find(agent => agent.id === 'codex');
    writeFileSync(codex.configPath, '[mcp_servers.other]\nurl = "http://other.test"\n');
    configureAgent(codex);
    const codexText = readFileSync(codex.configPath, 'utf8');
    assert.match(codexText, /\[mcp_servers\.other\]/);
    assert.match(codexText, /\[mcp_servers\.figma-rust-mcp\]\nurl = "http:\/\/127\.0\.0\.1:41730\/mcp"/);
    assert.equal(configureAgent(codex), false);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
});


test('tree reads preserve same-size screens and all repeated siblings', () => {
  const r = runtime();
  vm.runInContext(readFileSync('plugin-src/read-helpers.js', 'utf8'), r);
  const screens = Array.from({ length: 12 }, (_, i) => ({
    id: `screen:${i}`, name: `Screen ${i}`, type: 'FRAME', width: 402, height: 800,
    layoutMode: 'VERTICAL', itemSpacing: i,
    children: [{ id: `item:${i}`, name: `Item ${i}`, type: 'FRAME', width: 100, height: 40 }],
  }));
  const root = { id: 'root', name: 'Screens', type: 'SECTION', children: screens };
  for (const detail of ['compact', 'full']) {
    const tree = plain(r.extractDesignTree(root, 0, 10, detail, true));
    assert.equal(tree.children.length, 12);
    assert.deepEqual(tree.children.map(node => node.id), screens.map(node => node.id));
    assert.equal(tree.children[11].layout.itemSpacing, 11);
    assert.equal(tree.children[11].children[0].id, 'item:11');
  }
});


test('startup index defers document-wide components and coalesces duplicate scans', async () => {
  let componentQueries = 0, styleQueries = 0;
  const page = { id: 'page:1', children: [{ id: 'frame:1', name: 'Frame', type: 'FRAME' }] };
  const r = runtime({ currentPage: page, root: { name: 'File', getPluginData: () => '' } });
  r.handlers.get_styles = async () => { styleQueries++; return { schemaVersion: 2 }; };
  r.handlers.get_variables = async () => ({ schemaVersion: 2 });
  r.handlers.get_local_components = async () => { componentQueries++; return { components: [], componentSets: [] }; };
  const [a, b] = await Promise.all([
    r.handlers.index_scan({ deferComponents: true }),
    r.handlers.index_scan({ deferComponents: true }),
  ]);
  assert.equal(componentQueries, 0);
  assert.equal(styleQueries, 1);
  assert.equal(a, b);
  assert.equal(a.pageId, page.id);
  assert.equal(a.componentsDeferred, true);
  assert.equal(a.pageNodes[0].id, 'frame:1');
  const [light, full] = await Promise.all([
    r.handlers.index_scan({ deferComponents: true }), r.handlers.index_scan({}),
  ]);
  assert.equal(light.components, null);
  assert.equal(full.componentsDeferred, false);
  assert.equal(componentQueries, 1);
});

test('page index is shallow; frame reads paginate and instances expand only on request', async () => {
  const messages = [];
  let childReads = 0;
  const page = { id: 'page', type: 'PAGE', children: [] };
  const figma = { currentPage: page, skipInvisibleInstanceChildren: false,
    root: { name: 'File', getPluginData: () => '' }, ui: { postMessage: msg => messages.push(msg) } };
  const text = i => ({ id: 'text:' + i, name: 'Label ' + i, type: 'TEXT', characters: 'Content ' + i,
    width: 15.5, height: 20.25, visible: true,
    get fills() { assert.fail('projected index must not read paints'); },
    get fontSize() { assert.fail('projected index must not read typography'); } });
  const visible = Array.from({ length: 1200 }, (_, i) => text(i));
  const hidden = text('hidden'); hidden.visible = false;
  const frame = { id:'frame', name:'Frame', type:'FRAME', parent:page };
  const instance = { id:'instance', name:'Instance', type:'INSTANCE', parent:frame,
    get children() { childReads++; return figma.skipInvisibleInstanceChildren ? visible : [...visible,hidden]; } };
  for (const node of [...visible,hidden]) node.parent = instance;
  frame.children = [instance]; page.children = [frame];
  figma.getNodeByIdAsync = async id => id === frame.id ? frame : id === instance.id ? instance : null;
  const r = runtime(figma);
  r.handlers.get_styles = async () => ({}); r.handlers.get_variables = async () => ({});
  r.yieldToUI = async () => assert.equal(figma.skipInvisibleInstanceChildren, false);
  const startup = await r.handlers.index_scan({ deferComponents:true });
  assert.equal(startup.scope.id, page.id);
  assert.equal(startup.scope.depth, 0);
  assert.deepEqual(messages.filter(msg => msg.type === 'index-chunk').flatMap(msg => msg.nodes.map(n => n.id)), ['frame']);
  assert.equal(childReads, 0);
  const opaque = await r.handlers.read_nodes({ id:frame.id });
  assert.equal(opaque.nodes.length, 2); assert.equal(opaque.nodes[1].opaque, true); assert.equal(childReads, 0);
  messages.length = 0;
  const result = await r.handlers.index_scan({ id:frame.id, expandInstances:true, deferComponents:true });
  const chunks = messages.filter(msg => msg.type === 'index-chunk');
  assert.deepEqual(chunks.map(msg => msg.nodes.length), [500,500,202]);
  const nodes = chunks.flatMap(msg => plain(msg.nodes));
  assert.equal(childReads, 1); assert.equal(result.nodesTruncated, false);
  assert.equal(nodes.at(-1).content, 'Content 1199'); assert.equal(nodes.at(-1).width, 15.5);
  assert.equal(nodes[1].childIds.length, 1200);
  assert.equal(figma.skipInvisibleInstanceChildren, false);
  const first = await r.handlers.read_nodes({ id:frame.id, expandInstances:true, limit:2 });
  assert.ok(first.nextCursor); assert.equal(first.complete, false);
  const next = await r.handlers.read_nodes({ cursor:first.nextCursor, limit:2 });
  assert.deepEqual(plain(next.nodes.map(n => n.id)), ['text:0','text:1']);
  await assert.rejects(r.handlers.read_nodes({ cursor:first.nextCursor }), /expired/);
  await assert.rejects(r.handlers.read_nodes({ cursor:next.nextCursor, fields:['style'] }), /options cannot change/);
  r.nodeRevision++;
  await assert.rejects(r.handlers.read_nodes({ cursor:next.nextCursor }), /expired/);
  const full = await r.handlers.read_nodes({ id:instance.id, includeHidden:true, expandInstances:true, limit:500 });
  let all = [...full.nodes], cursor = full.nextCursor;
  while(cursor) { const page = await r.handlers.read_nodes({cursor,limit:500}); all.push(...page.nodes); cursor=page.nextCursor; }
  assert.equal(all.length, 1202); assert.equal(all.at(-1).visible, false);
  assert.throws(() => r.withInstanceVisibility(false, () => { throw Error('read failed'); }), /read failed/);
  assert.equal(figma.skipInvisibleInstanceChildren, false);
});

test('index yields by elapsed time and cancels when the active page changes', async () => {
  let elapsed = 0, reads = 0, previousReads = 0, yields = 0;
  const messages = [];
  const page = { id: 'page', children: Array.from({ length: 30 }, (_, i) => ({
    id: `${i}`, type: 'FRAME', get width() { elapsed += 2; reads++; return 10; },
  })) };
  const r = runtime({ skipInvisibleInstanceChildren: false, currentPage: page,
    root: { name: 'File', getPluginData: () => '' }, ui: { postMessage: msg => messages.push(msg) } });
  r.Date = { now: () => elapsed };
  r.handlers.get_styles = async () => ({});
  r.handlers.get_variables = async () => ({});
  r.yieldToUI = async () => {
    yields++;
    assert.ok(reads - previousReads <= 4, 'yield after an 8ms slice');
    previousReads = reads;
    assert.equal(r.figma.skipInvisibleInstanceChildren, false);
  };
  await r.handlers.index_scan({ deferComponents: true });
  assert.equal(reads, 30);
  assert.ok(yields >= 7);
  r.yieldToUI = async () => { r.figma.currentPage = { id: 'other', children: [] }; };
  messages.length = 0;
  const result = await r.handlers.index_scan({ deferComponents: true });
  assert.equal(result.cancelled, true);
  assert.ok(!messages.some(msg => msg.type === 'index-update'));
});

test('component catalogue walks pages cooperatively without bulk loading or root searches', async () => {
  let loaded = 0, yields = 0, reads = 0, lastYieldReads = 0, maxSliceReads = 0;
  const page = { type: 'PAGE', name: 'Components', loadAsync: async () => { loaded++; } };
  page.children = Array.from({ length: 1000 }, (_, i) => ({
    id: `c:${i}`, name: `Component ${i}`, parent: page, width: 10, height: 10,
    get type() { reads++; return i % 2 ? 'COMPONENT_SET' : 'COMPONENT'; },
  }));
  const r = runtime({
    root: { children: [page], findAllWithCriteria: () => assert.fail('root-wide search') },
    loadAllPagesAsync: () => assert.fail('bulk page load'),
  });
  r.yieldToUI = async () => {
    yields++; maxSliceReads = Math.max(maxSliceReads, reads - lastYieldReads); lastYieldReads = reads;
  };
  const result = await r.handlers.get_local_components();
  assert.equal(loaded, 1);
  assert.equal(result.components.length, 500);
  assert.equal(result.componentSets.length, 500);
  assert.ok(yields >= 10, 'large traversal must yield before finishing');
  assert.ok(maxSliceReads <= 200, 'no more than 100 nodes between traversal yields');
});

test('startup subscribes to active-page changes without loading the whole file', async () => {
  const events = new Map(), timers = [], messages = [];
  let pageListener;
  const page = {
    id: 'page:1', children: [], selection: [],
    on: (name, callback) => { assert.equal(name, 'nodechange'); pageListener = callback; },
    off: () => {},
  };
  const r = runtime({
    currentPage: page, root: { name: 'File', getPluginData: () => 'session' },
    clientStorage: { getAsync: async () => null }, ui: { postMessage: msg => messages.push(msg) },
    on: (name, callback) => { events.set(name, callback); },
    loadAllPagesAsync: () => assert.fail('startup bulk page load'),
  });
  r.setTimeout = (callback, delay) => { timers.push({ callback, delay }); return timers.length; };
  r.clearTimeout = () => {};
  r.__html__ = '<html>thin loader</html>';
  let reopenedUi = 0;
  r.figma.showUI = () => { reopenedUi++; };
  vm.runInContext(readFileSync('plugin-src/main.js', 'utf8'), r);
  assert.equal(reopenedUi, 0, 'dynamic runtime must not replace the loader iframe');
  assert.ok(pageListener);
  assert.ok(events.has('stylechange'));
  assert.ok(!events.has('documentchange'));
  let scanOptions;
  r.handlers.index_scan = async options => { scanOptions = options; return { pageId: page.id }; };
  await timers.find(timer => timer.delay === 1800).callback();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(scanOptions.deferComponents, true);
  pageListener({ nodeChanges: [{ type: 'PROPERTY_CHANGE', node: { id: 'changed:1' } }] });
  assert.ok(r.pendingChangedNodeIds.has('changed:1'));
  assert.ok(messages.some(msg => msg.type === 'nodes-invalidated'));
  const changed = { id:'changed:1', type:'TEXT', parent:page, width:15.5, characters:'New' };
  r.extractDesignTree = () => assert.fail('patch sync must not extract a tree');
  pageListener({ nodeChanges:[{ type:'PROPERTY_CHANGE', properties:['width','characters'], node:changed }] });
  pageListener({ nodeChanges:[{ type:'DELETE', id:'deleted:1' }] });
  await timers.filter(timer => timer.delay === 100).at(-1).callback();
  const diff = plain(messages.filter(msg => msg.type === 'node-patch').at(-1));
  assert.equal(diff.baseRevision, 0); assert.equal(diff.revision, 3);
  assert.equal(diff.patches[0].values.width, 15.5); assert.equal(diff.patches[0].values.content, 'New');
  assert.deepEqual(diff.patches[1], {kind:'delete',id:'deleted:1'});
  assert.equal(r.nodeReadCursors.size, 0);
  const next = { id: 'page:2', on: () => {}, off: () => {} };
  r.figma.currentPage = next;
  r.handlers.index_scan = async () => ({ pageId: 'page:1' });
  const previousUpdates = messages.filter(msg => msg.type === 'index-update').length;
  await r.publishIndex(true);
  assert.equal(messages.filter(msg => msg.type === 'index-update').length, previousUpdates);
});


function taskRuntime() {
  const page = { id: 'page', type: 'PAGE' };
  const a = { id: 'a', name: 'A', type: 'FRAME', parent: page, children: [] };
  const b = { id: 'b', name: 'B', type: 'FRAME', parent: page, children: [] };
  const child = { id: 'a:child', type: 'TEXT', parent: a };
  a.children.push(child);
  const nodes = new Map([page, a, b, child].map(node => [node.id, node]));
  const r = runtime();
  vm.runInContext(readFileSync('plugin-src/task-scope.js', 'utf8'), r);
  r.findNodeByIdAsync = async id => nodes.get(id);
  return { r, nodes, a, b, page };
}

test('frame tasks reject outside/global writes and batch escape before mutations', async () => {
  const { r, a, b } = taskRuntime();
  await r.handlers.task_start({ taskId: 'task-a', frameId: a.id });
  await r.handlers.task_start({ taskId: 'task-b', frameId: b.id });
  await assert.rejects(r.handlers.task_start({ taskId: 'task-c', frameId: a.id }), /reserved/);
  const create = await r.validateTaskOperation('create', { _taskId: 'task-a', type: 'TEXT', content: 'Hello' });
  assert.equal(create.params.parentId, a.id);
  const read = await r.validateTaskOperation('get_selection', { _taskId: 'task-a' });
  assert.equal(read.operation, 'get_design');
  assert.equal(read.params.id, a.id);
  assert.equal((await r.validateTaskOperation('screenshot', { _taskId: 'task-a', keepViewport: false })).params.keepViewport, true);
  await r.validateTaskOperation('modify', { _taskId: 'task-a', id: 'a:child', name: 'Updated' });
  for (const [operation, params] of [
    ['modify', { id: b.id }], ['create', { type: 'TEXT', parentId: b.id }],
    ['delete', { id: a.id, force: true }], ['clone', { id: a.id }],
    ['append', { parentId: b.id, childId: 'a:child' }],
    ['set_selection', { nodeIds: [a.id] }], ['setPage', { id: 'page' }],
    ['setVariableValue', { id: 'variable' }], ['createPaintStyle', { name: 'Global' }],
    ['create', { type: 'COMPONENT' }],
    ['batch', { operations: [{ operation: 'modify', params: { id: 'a:child' } }, { operation: 'delete', params: { id: b.id } }] }],
  ]) {
    await assert.rejects(r.validateTaskOperation(operation, { ...params, _taskId: 'task-a' }));
  }
  await assert.rejects(r.validateTaskOperation('modify', { id: 'a:child' }), /require taskId/);
  await r.handlers.task_end({ taskId: 'task-a' });
  await assert.rejects(r.validateTaskOperation('modify', { _taskId: 'task-a', id: 'a:child' }), /Unknown task/);
});

test('task roots cannot be shared masters or nested layout frames', async () => {
  const { r, nodes, a } = taskRuntime();
  const nested = { id: 'nested', type: 'FRAME', parent: a, children: [] };
  nodes.set(nested.id, nested);
  await assert.rejects(r.handlers.task_start({ taskId: 'nested-task', frameId: nested.id }), /independent FRAME/);
  a.children.push({ id: 'master', type: 'COMPONENT', parent: a });
  await assert.rejects(r.handlers.task_start({ taskId: 'master-task', frameId: a.id }), /shared component/);
});

test('plugin tab IDs differ for the same document and request replay does not write twice', async () => {
  const messages = [];
  function boot() {
    const r = runtime({
      fileKey: 'same-file', root: { name: 'File', getPluginData: () => 'same-file' },
      currentPage: { id: 'page', selection: [], on: () => {} },
      clientStorage: { getAsync: async () => null }, ui: { postMessage: msg => messages.push(msg) }, on: () => {},
    });
    r.setTimeout = () => 0; r.clearTimeout = () => {};
    r.validateTaskOperation = async (operation, params) => ({ operation, params });
    vm.runInContext(readFileSync('plugin-src/main.js', 'utf8'), r);
    return r;
  }
  const first = boot(), second = boot();
  assert.notEqual(first.currentSessionId, second.currentSessionId);
  assert.equal(first.currentDocumentId, second.currentDocumentId);
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const entered = [];
  first.handlers.modify = async params => { entered.push(params.id); if (params.id === 'first') await blocked; return params; };
  const call1 = first.figma.ui.onmessage({ id: 'request-1', operation: 'modify', params: { id: 'first' } });
  const replay = first.figma.ui.onmessage({ id: 'request-1', operation: 'modify', params: { id: 'first' } });
  const call2 = first.figma.ui.onmessage({ id: 'request-2', operation: 'modify', params: { id: 'second' } });
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(entered, ['first']);
  release(); await Promise.all([call1, replay, call2]);
  assert.deepEqual(entered, ['first', 'second']);
  assert.equal(messages.filter(msg => msg.id === 'request-1' && msg.success).length, 2);
  assert.ok(messages.filter(msg => msg.id).every(msg => msg.sessionId === first.currentSessionId));
});

test('a runtime hash change hot-reloads once, not on every later server-hello', () => {
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  const source = html.slice(html.indexOf('    function connectWs()'), html.indexOf('    async function startLongPoll()'));
  const sockets = [];
  let reloads = 0;
  class Socket {
    static OPEN = 1;
    constructor(url) { this.url = url; this.readyState = 1; sockets.push(this); }
    close() {} send() {}
  }
  const r = vm.createContext({
    uiDisposed: false, ws: null, wsConnected: false, sessionId: 'tab-a', fileName: 'File', documentId: 'doc', BRIDGE: 'http://localhost:41730',
    WebSocket: Socket, setInterval: () => 1, clearInterval: () => {}, setTimeout: () => { reloads++; return 1; }, clearTimeout: () => {},
    sendRuntimeCapabilities: () => {}, consecutiveErrors: 0, everConnected: false, retryTimer: null, polling: true,
    setStatus: () => {}, log: () => {}, currentPort: 41730, READ_OPS: [], updateRuntimeVersion: () => {},
    activeIndexScan: null, startLongPoll: () => {}, dispatchToMain: () => {}, window: {},
  });
  vm.runInContext(source, r);
  r.connectWs(); sockets[0].onopen();
  const hello = hash => sockets[0].onmessage({ data: JSON.stringify({ type: 'server-hello', version: '5.1.0', runtimeHash: hash }) });
  hello('a'); assert.equal(reloads, 0);
  hello('b'); assert.equal(reloads, 1);
  // window survives the reload's document.write; the same server must not reload it again.
  hello('b'); hello('b'); assert.equal(reloads, 1);
});

test('old UI socket callbacks cannot clear or dispatch through a replacement socket', () => {
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  const source = html.slice(html.indexOf('    function connectWs()'), html.indexOf('    async function startLongPoll()'));
  const sockets = [], dispatched = [];
  class Socket {
    static OPEN = 1;
    constructor(url) { this.url = url; this.readyState = 1; sockets.push(this); }
    close() {} send() {}
  }
  const r = vm.createContext({
    uiDisposed: false, ws: null, wsConnected: false, sessionId: null, fileName: 'File', documentId: 'doc', BRIDGE: 'http://localhost:41730',
    WebSocket: Socket, setInterval: () => 1, clearInterval: () => {}, setTimeout: () => 1, clearTimeout: () => {},
    sendRuntimeCapabilities: () => {}, consecutiveErrors: 0, everConnected: false, retryTimer: null, polling: true,
    setStatus: () => {}, log: () => {}, currentPort: 41730, READ_OPS: [],
    activeIndexScan: null, startLongPoll: () => assert.fail('stale socket started poll'), dispatchToMain: req => dispatched.push(req),
  });
  vm.runInContext(source, r);
  r.connectWs(); assert.equal(sockets.length, 0);
  r.sessionId = 'tab-a'; r.connectWs(); const old = sockets[0];
  old.onopen(); r.connectWs(); const replacement = sockets[1]; replacement.onopen();
  old.onclose(); old.onmessage({ data: JSON.stringify({ id: 'old', operation: 'modify' }) });
  assert.equal(r.wsConnected, true);
  assert.equal(r.ws, replacement);
  assert.equal(dispatched.length, 0);
  replacement.onmessage({ data: JSON.stringify({ id: 'new', operation: 'modify' }) });
  assert.equal(dispatched[0].id, 'new');
});

test('tree reads preserve Regular font weight, collect Regular font tokens, and resolve textStyle', async () => {
  const figma = {
    getLocalPaintStylesAsync: async () => [{ id: 'S:paint-1', name: 'Brand/Primary' }],
    getLocalTextStylesAsync: async () => [{ id: 'S:text-1', name: 'Lato/18px/regular' }],
    getLocalEffectStylesAsync: async () => [],
  };
  const context = vm.createContext({ figma, handlers: {}, console, setTimeout, clearTimeout, Map, Set, Uint8Array });
  for (const file of ['utils', 'svg-path-helpers', 'paint-and-effects', 'token-helpers', 'read-helpers', 'handlers-read-detail', 'handlers-read']) {
    vm.runInContext(readFileSync(`plugin-src/${file}.js`, 'utf8'), context, { filename: file });
  }

  const walkState = await context.makeWalkStateAsync();
  const tokenCollector = { colors: new Set(), fonts: new Set(), sizes: new Set() };
  const textNode = {
    id: '3139:247224',
    name: 'Header',
    type: 'TEXT',
    characters: 'Shipping address',
    fontSize: 18,
    fontName: { family: 'Lato', style: 'Regular' },
    textStyleId: 'S:text-1',
    visible: true,
  };
  const tree = context.extractDesignTree(textNode, 0, 15, 'full', true, tokenCollector, null, walkState);
  assert.equal(tree.fontWeight, 'Regular');
  assert.equal(tree.fontFamily, 'Lato');
  assert.equal(tree.fontSize, 18);
  assert.equal(tree.textStyleId, 'S:text-1');
  assert.equal(tree.textStyle, 'Lato/18px/regular');
  assert.ok(tokenCollector.fonts.has('Lato/Regular/18px'));
});

test('hot reload removes old Figma listeners and timers before installing the next runtime', async () => {
  const events = new Map(), pageEvents = new Set(), timers = new Map(), messages = [];
  let timerId = 0;
  const page = { id: 'page', children: [], selection: [], on: (_, fn) => pageEvents.add(fn), off: (_, fn) => pageEvents.delete(fn) };
  let styleName = 'Before';
  const figma = { getLocalTextStylesAsync: async () => [{ id: 'text-style', name: styleName }], currentPage: page, root: { name: 'File', getPluginData: () => 'document' },
    clientStorage: { getAsync: async () => null }, ui: { postMessage: msg => messages.push(msg) },
    on: (event, fn) => { if (!events.has(event)) events.set(event, new Set()); events.get(event).add(fn); },
    off: (event, fn) => events.get(event)?.delete(fn) };
  const main = readFileSync('plugin-src/main.js', 'utf8');
  const runtimes = [];
  for (let i = 0; i < 3; i++) {
    const r = runtime(figma);
    r.setTimeout = (fn, delay) => { timers.set(++timerId, { fn, delay }); return timerId; };
    r.clearTimeout = id => timers.delete(id);
    r.__html__ = '';
    vm.runInContext(main, r);
    runtimes.push(r);
    for (const listeners of events.values()) assert.equal(listeners.size, 1);
    assert.equal(pageEvents.size, 1);
    assert.equal(timers.size, 1, 'only the current startup timer survives');
  }
  assert.equal(runtimes[0].runtimeDisposed, true);
  messages.length = 0;
  for (const fn of events.get('selectionchange')) fn();
  assert.equal(messages.filter(msg => msg.type === 'selection-change').length, 1);
  const live = runtimes.at(-1);
  assert.equal((await live.getStyleNameMapAsync())['text-style'], 'Before');
  styleName = 'After';
  for (const fn of events.get('stylechange')) fn({});
  assert.equal((await live.getStyleNameMapAsync())['text-style'], 'After');
  let releaseStyles;
  live.handlers.get_styles = () => new Promise(resolve => { releaseStyles = resolve; });
  live.handlers.get_variables = async () => ({});
  const scan = live.handlers.index_scan({ deferComponents: true });
  while (!releaseStyles) await new Promise(resolve => setTimeout(resolve, 0));
  figma.ui.onmessage.dispose();
  messages.length = 0;
  releaseStyles({});
  assert.equal((await scan).cancelled, true);
  assert.ok(!messages.some(msg => msg.type === 'index-update'));
  assert.equal(timers.size, 0);
  assert.equal(pageEvents.size, 0);
  for (const listeners of events.values()) assert.equal(listeners.size, 0);
});

test('style names share in-flight reads and refresh after invalidation or a failed getter', async () => {
  const calls = {};
  let name = 'Before', failPaint = false;
  const figma = Object.fromEntries(['Paint', 'Text', 'Effect', 'Grid'].map(kind => [`getLocal${kind}StylesAsync`, async () => {
    calls[kind] = (calls[kind] || 0) + 1;
    if (kind === 'Paint' && failPaint) throw new Error('temporary failure');
    return [{ id: kind, name }];
  }]));
  const r = runtime(figma);
  const [first, second] = await Promise.all([r.makeWalkStateAsync({}), r.makeWalkStateAsync({})]);
  assert.equal(first.styleMap.Text, 'Before');
  assert.equal(second.styleMap.Paint, 'Before');
  assert.deepEqual(Object.values(calls), [1, 1, 1, 1]);
  name = 'After'; r.invalidateStyleNameMap();
  assert.equal((await r.makeWalkStateAsync({})).styleMap.Text, 'After');
  assert.deepEqual(Object.values(calls), [2, 2, 2, 2]);
  failPaint = true; r.invalidateStyleNameMap();
  assert.equal((await r.makeWalkStateAsync({})).styleMap.Paint, undefined);
  failPaint = false;
  assert.equal((await r.makeWalkStateAsync({})).styleMap.Paint, 'After');
  assert.equal(calls.Paint, 4);
  let release;
  r.figma.getLocalPaintStylesAsync = () => new Promise(resolve => { release = resolve; });
  r.invalidateStyleNameMap();
  const old = r.getStyleNameMapAsync();
  r.invalidateStyleNameMap();
  r.figma.getLocalPaintStylesAsync = async () => [{ id: 'Paint', name: 'Newest' }];
  assert.equal((await r.getStyleNameMapAsync()).Paint, 'Newest');
  release([{ id: 'Paint', name: 'Obsolete' }]);
  await old;
  assert.equal((await r.getStyleNameMapAsync()).Paint, 'Newest');
});

for (const lineEnding of ['\n', '\r\n']) test(`UI reload clears previous timers, listeners and socket while keeping inline controls available (${lineEnding === '\n' ? 'LF' : 'CRLF'})`, () => {
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  const source = html.match(/<script>([\s\S]*?)<\/script>/)[1].replace(/\r?\n/g, lineEnding);
  const timers = new Set();
  let timerId = 0, socketCloses = 0, cancelledRaf = 0;
  const elements = new Map();
  const target = () => {
    const listeners = new Map();
    return {
      listeners, style: {}, dataset: {}, classList: { add() {}, remove() {}, contains: () => false, toggle() {} },
      addEventListener(name, fn, options) {
        if (options?.signal?.aborted) return;
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(fn);
        options?.signal?.addEventListener('abort', () => listeners.get(name).delete(fn), { once: true });
      },
      removeEventListener(name, fn) { listeners.get(name)?.delete(fn); },
      setPointerCapture() {}, releasePointerCapture() {},
      setAttribute() {}, querySelectorAll: () => [], querySelector: () => null,
    };
  };
  const window = { ...target(), innerWidth: 360, innerHeight: 600,
    setTimeout() { timers.add(++timerId); return timerId; }, clearTimeout: id => timers.delete(id),
    setInterval() { timers.add(++timerId); return timerId; }, clearInterval: id => timers.delete(id),
  };
  const document = { ...target(), getElementById: id => {
    if (!elements.has(id)) elements.set(id, target());
    return elements.get(id);
  }, querySelectorAll: () => [] };
  class Socket {
    static OPEN = 1;
    constructor() { this.readyState = 1; }
    close() { socketCloses++; }
    send() {}
  }
  const r = vm.createContext({ window, document, WebSocket: Socket, AbortController, AbortSignal,
    console, navigator: {}, fetch: () => new Promise(() => {}), parent: { postMessage() {} },
    requestAnimationFrame: () => 42, cancelAnimationFrame: () => cancelledRaf++ });
  for (let i = 0; i < 3; i++) {
    const entryPoint = /^( {4})initConnection\(\);(?=\r?$)/m;
    assert.match(source, entryPoint, 'startup entry point must be replaced by the socket fixture');
    vm.runInContext(source.replace(entryPoint, "$1sessionId = 's'; connectWs();"), r);
    assert.equal(timers.size, 1, 'only current stats interval remains');
    assert.equal(window.listeners.get('focus').size, 1);
    assert.equal(document.listeners.get('keydown').size, 1);
    assert.equal(document.listeners.get('visibilitychange').size, 1);
    for (const name of [...html.matchAll(/onclick="([A-Za-z][A-Za-z0-9_]*)\(/g)].map(match => match[1]).filter(name => name !== 'document')) {
      assert.equal(typeof window[name], 'function', `${name} must remain available to inline controls`);
    }
  }
  // Cleanup also covers a resize in progress.
  const grip = elements.get('resize-grip');
  for (const fn of grip.listeners.get('pointerdown')) fn({ preventDefault() {}, pointerId: 1, clientX: 0, clientY: 0 });
  for (const fn of window.listeners.get('pointermove')) fn({ clientX: 10, clientY: 10 });
  window.__disposeFigmaMcpUi();
  assert.equal(timers.size, 0);
  assert.equal(cancelledRaf, 1);
  for (const listeners of window.listeners.values()) assert.equal(listeners.size, 0);
  for (const listeners of document.listeners.values()) assert.equal(listeners.size, 0);
  window.__disposeFigmaMcpUi();
  assert.equal(cancelledRaf, 1, 'cleanup is idempotent');
  assert.equal(socketCloses, 3);
});


test('creating a paint style invalidates the style names before the next design read', async () => {
  const styles = [];
  const r = runtime({ getLocalPaintStylesAsync: async () => styles,
    createPaintStyle: () => { const style = { id: 'new-style' }; styles.push(style); return style; } });
  assert.equal((await r.getStyleNameMapAsync())['new-style'], undefined);
  await r.handlers.createPaintStyle({ name: 'Brand/New', color: '#ff0000' });
  assert.equal((await r.getStyleNameMapAsync())['new-style'], 'Brand/New');
});

test('byte-bounded reads paginate without dropping unicode, oversized nodes or descendants', async () => {
  const page = {id:'page',type:'PAGE'};
  const frame = {id:'frame',name:'Frame',type:'FRAME',parent:page};
  frame.children = Array.from({length:12},(_,i)=>({id:`text:${i}`,name:'Label',type:'TEXT',parent:frame,
    characters:i === 4 ? '漢'.repeat(2000) : 'é'.repeat(100),visible:true}));
  page.children = [frame];
  const r = runtime({root:{id:"root"},currentPage:page,getNodeByIdAsync:async()=>frame});
  let read = await r.handlers.read_nodes({id:'frame',expandInstances:true,maxBytes:1024,limit:500});
  const all = [...read.nodes]; let oversized = false, pages = 1;
  while(read.nextCursor) {
    assert.ok(pages++ < 20);
    read = await r.handlers.read_nodes({cursor:read.nextCursor,maxBytes:1024,limit:500});
    oversized ||= read.oversizedNode; all.push(...read.nodes);
  }
  assert.equal(read.complete,true); assert.equal(oversized,true);
  assert.deepEqual(plain(all.map(n=>n.id)),['frame',...frame.children.map(n=>n.id)]);
  assert.equal(all[5].content,'漢'.repeat(2000));
  await assert.rejects(r.handlers.read_nodes({id:'frame',maxBytes:0}),/maxBytes/);
});

test('letter spacing PERCENT converts to px; line-height percent keeps decimals', () => {
  const r = runtime();
  const node = { id: '1:1', type: 'TEXT', characters: 'Hi', fontSize: 20, fontName: { family: 'Inter', style: 'Regular' },
    letterSpacing: { unit: 'PERCENT', value: -2.5 }, lineHeight: { unit: 'PERCENT', value: 137.456 } };
  const style = r.resolveTextStyle(node);
  assert.equal(style.letterSpacing, -0.5);
  assert.equal(style.lineHeight, '137.46%');
  assert.equal(r.resolveTextStyle({ ...node, letterSpacing: { unit: 'PIXELS', value: 1.2 } }).letterSpacing, 1.2);
});

test('blur effects emit half the Figma radius', () => {
  const r = runtime();
  const css = r.effectsToCss([{ type: 'LAYER_BLUR', radius: 8 }, { type: 'BACKGROUND_BLUR', radius: 20 }]);
  assert.equal(css.filter, 'blur(4px)');
  assert.equal(css.backdropFilter, 'blur(10px)');
});

test('findNodeByIdAsync returns null for a missing instance sublayer, not its parent instance', async () => {
  const instance = { id: '2715:40862', type: 'INSTANCE' };
  const r = runtime({ root: { id: '0:0' }, currentPage: { id: '0:1', selection: [] },
    getNodeByIdAsync: async id => (id === instance.id ? instance : null) });
  assert.equal(await r.findNodeByIdAsync('I2715:40862;123:456'), null);
  assert.equal(await r.findNodeByIdAsync('2715:40862'), instance);
});

test('bound variables resolve in the node mode and report library aliases', async () => {
  const cols = [{ id: 'sem', defaultModeId: 'light' }, { id: 'lib', defaultModeId: 'l1' }];
  const local = [{ id: 'bg', name: 'bg/surface', variableCollectionId: 'sem', resolvedType: 'COLOR',
    valuesByMode: { light: { type: 'VARIABLE_ALIAS', id: 'white' }, dark: { type: 'VARIABLE_ALIAS', id: 'black' } } }];
  const library = [
    { id: 'white', name: 'gray/0', variableCollectionId: 'lib', resolvedType: 'COLOR', valuesByMode: { l1: { r: 1, g: 1, b: 1, a: 1 } } },
    { id: 'black', name: 'gray/900', variableCollectionId: 'lib', resolvedType: 'COLOR', valuesByMode: { l1: { r: 0, g: 0, b: 0, a: 1 } } },
  ];
  const r = runtime({ variables: {
    getLocalVariablesAsync: async () => local, getLocalVariableCollectionsAsync: async () => cols.slice(0, 1),
    getVariableByIdAsync: async id => library.find(v => v.id === id) || local.find(v => v.id === id),
    getVariableCollectionByIdAsync: async id => cols.find(c => c.id === id),
  } });
  const node = { id: '1:1', name: 'Card', type: 'RECTANGLE', width: 10, height: 10, x: 0, y: 0,
    fills: [{ type: 'SOLID', color: { r: 0, g: 0, b: 0 } }], boundVariables: { fills: [{ id: 'bg' }] },
    resolvedVariableModes: { sem: 'dark' } };
  const map = await r.buildVariableResolverMapAsync(true);
  const walkState = { variableMap: map, remaining: 10, budget: 10 };
  const tree = r.extractDesignTree(node, 0, 5, 'full', true, null, null, walkState);
  await r.resolvePendingVariablesAsync(walkState);
  const entry = plain(tree.boundVariables.fills[0]);
  assert.equal(entry.name, 'bg/surface');
  assert.equal(entry.aliasTarget, 'gray/900');
  assert.equal(entry.value, '#000000');
  assert.equal(tree.fillToken, 'bg/surface');
  // Second walk: library variables are cached, resolved synchronously.
  const light = r.extractDesignTree({ ...node, resolvedVariableModes: { sem: 'light' } }, 0, 5, 'full', true, null, null, { variableMap: map, remaining: 10, budget: 10 });
  assert.equal(light.boundVariables.fills[0].value, '#ffffff');
  assert.equal(light.boundVariables.fills[0].aliasTarget, 'gray/0');
});

test('resolveVariableValueAsync reports alias names for node consumers', async () => {
  const cols = [{ id: 'sem', defaultModeId: 'light' }, { id: 'prim', defaultModeId: 'p' }];
  const vars = [
    { id: 'a', name: 'text/primary', variableCollectionId: 'sem', resolvedType: 'COLOR', valuesByMode: { light: { type: 'VARIABLE_ALIAS', id: 'b' }, dark: { type: 'VARIABLE_ALIAS', id: 'c' } },
      resolveForConsumer: () => ({ value: { r: 1, g: 1, b: 1, a: 1 } }) },
    { id: 'b', name: 'gray/900', variableCollectionId: 'prim', resolvedType: 'COLOR', valuesByMode: { p: { r: 0, g: 0, b: 0, a: 1 } } },
    { id: 'c', name: 'gray/0', variableCollectionId: 'prim', resolvedType: 'COLOR', valuesByMode: { p: { r: 1, g: 1, b: 1, a: 1 } } },
  ];
  const r = runtime({ variables: { getVariableByIdAsync: async id => vars.find(v => v.id === id), getVariableCollectionByIdAsync: async id => cols.find(c => c.id === id) } });
  const res = await r.resolveVariableValueAsync(vars[0], { resolvedVariableModes: { sem: 'dark', prim: 'p' } });
  assert.equal(res.type, 'ALIAS');
  assert.equal(res.primitiveName, 'gray/0');
  assert.equal(res.hex, '#ffffff');
});

test('get_variables finishes when Figma never answers a library alias lookup', async () => {
  const sem = { id: 'V:2', name: 'bg/primary', resolvedType: 'COLOR', variableCollectionId: 'C:2',
    valuesByMode: { l: { type: 'VARIABLE_ALIAS', id: 'VariableID:lib/1' } }, scopes: [] };
  const col = { id: 'C:2', name: 'Sem', defaultModeId: 'l', modes: [{ modeId: 'l', name: 'Light' }], variableIds: ['V:2'] };
  const r = runtime({ variables: {
    getLocalVariableCollectionsAsync: async () => [col], getLocalVariablesAsync: async () => [sem],
    getVariableByIdAsync: () => new Promise(() => {}), getVariableCollectionByIdAsync: async () => col } });
  const out = plain(await r.handlers.get_variables());
  assert.equal(out.collections[0].variables[0].values.l.resolvedValue, null);
  assert.ok(out.diagnostics.some(d => /did not answer variable VariableID:lib\/1/.test(d.message)), JSON.stringify(out.diagnostics));
});
