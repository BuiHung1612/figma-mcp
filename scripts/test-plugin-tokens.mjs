import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { spawnSync } from 'node:child_process';

function runtime(figma = {}) {
  const context = vm.createContext({ figma, handlers: {}, console, setTimeout, Map, Set, Uint8Array });
  for (const file of ['utils', 'token-helpers', 'handlers-read-detail', 'handlers-read', 'handlers-tokens', 'handlers-write-ops']) {
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

test('multiple paints preserve top-first ordering and node opacity stays separate', () => {
  const r = runtime();
  const paints = [{ type: 'SOLID', color: { r: 1, g: 0, b: 0 }, opacity: .5 }, { type: 'SOLID', color: { r: 0, g: 0, b: 1 }, opacity: 1 }].map(r.serializePaint);
  const css = r.paintsToCss(paints, 200, 100);
  assert.match(css.value, /^linear-gradient\(rgba\(255, 0, 0, 0.5\)/);
  assert.match(css.value, /linear-gradient\(#0000ff, #0000ff\)$/);
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
  assert.ok(messages.some(msg => msg.type === 'document-change'));
  assert.deepEqual(plain(messages.filter(msg => msg.type === 'document-change').at(-1).changedNodeIds), ['changed:1']);
  r.findNodeByIdAsync = async id => id === 'changed:1'
    ? { id, parent: page, type: 'TEXT', fontSize: 15.5, fontWeight: 'Semi Bold' } : null;
  r.extractDesignTree = node => ({ id: node.id, type: node.type, fontSize: node.fontSize, fontWeight: node.fontWeight });
  pageListener({ nodeChanges: [{ type: 'DELETE', id: 'deleted:1' }] });
  await timers.filter(timer => timer.delay === 100).at(-1).callback();
  const diff = plain(messages.filter(msg => msg.type === 'node-diff').at(-1));
  assert.deepEqual(diff.deletedIds, ['deleted:1']);
  assert.equal(diff.nodes[0].fontSize, 15.5);
  assert.equal(diff.nodes[0].fontWeight, 'Semi Bold');
  assert.equal(diff.nodes[0].parentId, page.id);
  assert.deepEqual(diff.nodes[0].childIds, []);
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
    ws: null, wsConnected: false, sessionId: null, fileName: 'File', documentId: 'doc', BRIDGE: 'http://localhost:38451',
    WebSocket: Socket, setInterval: () => 1, clearInterval: () => {}, setTimeout: () => 1, clearTimeout: () => {},
    sendRuntimeCapabilities: () => {}, consecutiveErrors: 0, everConnected: false, retryTimer: null, polling: true,
    setStatus: () => {}, log: () => {}, currentPort: 38451, READ_OPS: [],
    startLongPoll: () => assert.fail('stale socket started poll'), dispatchToMain: req => dispatched.push(req),
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
  const context = vm.createContext({ figma, handlers: {}, console, setTimeout, Map, Set, Uint8Array });
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
