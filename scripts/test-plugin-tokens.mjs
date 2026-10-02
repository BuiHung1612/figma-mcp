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
