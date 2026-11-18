// Write-path checks for plugin-src handlers against a minimal in-memory Figma tree.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

let seq = 0;
const CONTAINERS = ['FRAME', 'PAGE', 'INSTANCE', 'COMPONENT', 'COMPONENT_SET'];
function node(type, props = {}) {
  const data = {};
  const n = { id: `9:${++seq}`, type, name: props.name || type, parent: null, removed: false, x: 0, y: 0, width: 10, height: 10,
    visible: true, opacity: 1, fills: [], strokes: [], effects: [], ...props };
  delete n.children;
  if (props.children !== undefined || CONTAINERS.includes(type)) {
    n.children = [];
    if (type !== 'PAGE') n.layoutMode = props.layoutMode || 'NONE';
    for (const child of props.children || []) { child.parent = n; n.children.push(child); }
    n.insertChild = (i, child) => {
      if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
      child.parent = n;
      n.children.splice(i, 0, child);
    };
    n.appendChild = child => n.insertChild(n.children.length, child);
    n.findAll = fn => n.children.flatMap(c => [c, ...(c.findAll ? c.findAll(() => true) : [])]).filter(fn);
    n.findOne = fn => n.findAll(fn)[0] || null;
  }
  n.getPluginData = k => data[k] || '';
  n.setPluginData = (k, v) => { data[k] = v; };
  n.resize = (w, h) => { n.width = w; n.height = h; };
  n.remove = () => { n.removed = true; if (n.parent) n.parent.children.splice(n.parent.children.indexOf(n), 1); };
  n.clone = () => {
    const copy = node(type, { ...props, name: n.name, children: (n.children || []).map(c => c.clone()) });
    if (n.getPluginData('mcpKey')) copy.setPluginData('mcpKey', n.getPluginData('mcpKey'));
    n.parent && n.parent.appendChild(copy);
    return copy;
  };
  return n;
}

const FONTS = [['Inter', 'Regular'], ['Inter', 'Semi Bold'], ['Avenir Next', 'Regular'], ['Avenir Next', 'Demi Bold']];
function runtime(page) {
  page.selection = [];
  const all = () => [page, ...page.findAll(() => true)];
  const figma = {
    root: { id: '0:0' }, currentPage: page, ui: { postMessage() {} },
    getNodeByIdAsync: async id => all().find(n => n.id === id && !n.removed) || null,
    listAvailableFontsAsync: async () => FONTS.map(([family, style]) => ({ fontName: { family, style } })),
    loadFontAsync: async ({ family, style }) => {
      if (!FONTS.some(([f, s]) => f === family && s === style)) throw new Error('not found');
    },
    commitUndo() { figma.commits = (figma.commits || 0) + 1; },
    createFrame: () => page.appendChild(node('FRAME')) || page.children.at(-1),
    createText: () => { const t = node('TEXT'); page.appendChild(t); return t; },
    createRectangle: () => { const r = node('RECTANGLE'); page.appendChild(r); return r; },
  };
  // handlers-write.js declares `const handlers`, so read it back from the context.
  const context = vm.createContext({ figma, console, setTimeout, clearTimeout, Map, Set });
  for (const file of ['utils', 'paint-and-effects', 'handlers-write', 'handlers-write-ops', 'node-reader']) {
    vm.runInContext(readFileSync(`plugin-src/${file}.js`, 'utf8'), context, { filename: file });
  }
  return { figma, page, handlers: vm.runInContext('handlers', context) };
}

test('create and modify reject unknown and removed keys before changing anything', async () => {
  const box = node('FRAME');
  const r = runtime(node('PAGE', { children: [box] }));
  await assert.rejects(r.handlers.modify({ id: box.id, opacity: 0.5, letterSpacing: 2 }), /modify: unsupported key letterSpacing/);
  assert.equal(box.opacity, 1);
  await assert.rejects(r.handlers.modify({ id: box.id, layoutAlign: 'STRETCH' }), /layoutAlign was removed: use layoutSizingHorizontal\/Vertical: "FILL"/);
  await assert.rejects(r.handlers.create({ type: 'FRAME', primaryAxisSizingMode: 'AUTO' }), /primaryAxisSizingMode was removed/);
  assert.equal(r.page.children.length, 1);
  await assert.rejects(r.handlers.modify({ id: box.id, fontSize: 12 }), /fontSize do not apply to FRAME/);
});

test('create honours fontFamily, maps weights per family and never swaps fonts', async () => {
  const r = runtime(node('PAGE'));
  await r.handlers.create({ type: 'TEXT', content: 'Hi', fontFamily: 'Avenir Next', fontWeight: 'SemiBold' });
  assert.deepEqual({ ...r.page.children[0].fontName }, { family: 'Avenir Next', style: 'Demi Bold' });
  await assert.rejects(r.handlers.create({ type: 'TEXT', content: 'Hi', fontFamily: 'Avenir Next', fontWeight: 'Black' }), /has no Black style. Available: Regular, Demi Bold/);
  await assert.rejects(r.handlers.create({ type: 'TEXT', content: 'Hi', fontFamily: 'Comic' }), /not installed/);
  assert.equal(r.page.children.length, 1);
});

test('layoutSizing is the one sizing control and FILL wins over text auto-resize', async () => {
  const row = node('FRAME', { layoutMode: 'HORIZONTAL' });
  const r = runtime(node('PAGE', { children: [row] }));
  const text = await r.handlers.create({ type: 'TEXT', content: 'x', parentId: row.id, width: 100, layoutSizingHorizontal: 'FILL' });
  const t = row.children[0];
  assert.equal(t.id, text.id);
  assert.deepEqual([t.textAutoResize, t.layoutSizingHorizontal], ['HEIGHT', 'FILL']);
  const hug = await r.handlers.create({ type: 'FRAME', layoutMode: 'VERTICAL', parentId: row.id });
  const f = row.children[1];
  assert.equal(f.id, hug.id);
  assert.deepEqual([f.layoutSizingHorizontal, f.layoutSizingVertical], ['HUG', 'HUG']);
  await assert.rejects(r.handlers.create({ type: 'RECTANGLE', layoutSizingHorizontal: 'FILL' }), /"FILL" needs an auto-layout parent/);
  const warned = await r.handlers.create({ type: 'RECTANGLE', parentId: row.id, x: 5, y: 5 });
  assert.match(warned.warnings[0], /x\/y ignored/);
});

test('modify reparents, floats with ABSOLUTE and clears fills', async () => {
  const anchor = node('FRAME', { name: 'anchor' });
  const list = node('FRAME', { name: 'list', layoutMode: 'VERTICAL', children: [node('FRAME'), anchor] });
  const dropdown = node('FRAME', { name: 'dropdown', fills: [{ type: 'SOLID' }] });
  const page = node('PAGE', { children: [list, dropdown] });
  const r = runtime(page);
  const info = await r.handlers.modify({ id: dropdown.id, parentId: list.id, insertIndex: 0, layoutPositioning: 'ABSOLUTE', x: 16, y: 40, fills: [] });
  assert.equal(list.children[0], dropdown);
  assert.equal(info.layoutPositioning, 'ABSOLUTE');
  assert.deepEqual([dropdown.x, dropdown.y, dropdown.fills.length], [16, 40, 0]);
  assert.deepEqual([...r.handlers && (await r.handlers.query({ id: list.id }))[0].childIds], list.children.map(c => c.id));
  await assert.rejects(r.handlers.modify({ id: list.id, parentId: anchor.id }), /itself or its descendant/);
  await assert.rejects(r.handlers.modify({ id: anchor.id, parentId: page.id, layoutPositioning: 'ABSOLUTE' }), /auto-layout parent/);
});

test('modify applies text sizing, alignment and decoration', async () => {
  const text = node('TEXT', { fontName: { family: 'Inter', style: 'Regular' }, characters: 'Hi' });
  const r = runtime(node('PAGE', { children: [text] }));
  await r.handlers.modify({ id: text.id, textAutoResize: 'height', textAlignHorizontal: 'left', textDecoration: 'underline' });
  assert.deepEqual([text.textAutoResize, text.textAlignHorizontal, text.textDecoration], ['HEIGHT', 'LEFT', 'UNDERLINE']);
});

test('batch is all-or-nothing and returns results in order', async () => {
  const r = runtime(node('PAGE'));
  const ok = await r.handlers.batch({ operations: [{ operation: 'create', params: { type: 'FRAME', key: 'a' } }] });
  assert.equal(ok[0].key, 'a');
  await assert.rejects(r.handlers.batch({ operations: [
    { operation: 'create', params: { type: 'FRAME' } },
    { operation: 'create', params: { type: 'FRAME', bogus: 1 } },
  ] }), /batch op 1 \(create\) failed: .*bogus.*removed the 1 node/);
  assert.equal(r.page.children.length, 1);
});

test('rollback removes what a failed call created; commit_undo starts a new step', async () => {
  const r = runtime(node('PAGE'));
  await r.handlers.create({ type: 'FRAME' });
  await r.handlers.commit_undo({});
  assert.equal(r.figma.commits, 1);
  await r.handlers.create({ type: 'FRAME' });
  await r.handlers.create({ type: 'RECTANGLE' });
  assert.deepEqual({ ...(await r.handlers.rollback({})) }, { removed: 2 });
  assert.equal(r.page.children.length, 1);
});

test('render builds a keyed tree in one call and removes it all on failure', async () => {
  const r = runtime(node('PAGE'));
  const out = await r.handlers.render({ tree: { type: 'FRAME', key: 'sheet', layoutMode: 'VERTICAL', children: [
    { type: 'TEXT', key: 'title', content: 'Search location' },
    { type: 'FRAME', key: 'row', layoutMode: 'HORIZONTAL', layoutSizingHorizontal: 'FILL' },
  ] } });
  assert.deepEqual(Object.keys(out.ids), ['sheet', 'title', 'row']);
  assert.equal(out.root.id, out.ids.sheet);
  assert.equal((await r.handlers.find_by_key({ key: 'title' })).id, out.ids.title);
  assert.equal(await r.handlers.find_by_key({ key: 'nope' }), null);
  await assert.rejects(r.handlers.render({ tree: { type: 'FRAME', children: [{ type: 'TEXT', content: 'x' }, { type: 'NOPE' }] } }), /Unsupported node type/);
  assert.equal(r.page.children.length, 1);
});

test('setComponentProperties resolves names on variant instances without reading the main definitions', async () => {
  let applied;
  const inst = node('INSTANCE', {
    componentProperties: { 'Label#132:39': { type: 'TEXT', value: 'Old' }, 'State': { type: 'VARIANT', value: 'Default' } },
    setProperties: props => { applied = props; },
    getMainComponentAsync: async () => ({ get componentPropertyDefinitions() { throw new Error('Can only get component property definitions of a component set'); } }),
  });
  const r = runtime(node('PAGE', { children: [inst] }));
  await r.handlers.setComponentProperties({ id: inst.id, properties: { Label: 'New', State: 'Focused' } });
  assert.deepEqual({ ...applied }, { 'Label#132:39': 'New', State: 'Focused' });
});

test('clone maps ids, drops copied keys and detach returns a plain frame', async () => {
  const leaf = node('TEXT');
  const src = node('FRAME', { children: [node('FRAME', { children: [leaf] })] });
  src.setPluginData('mcpKey', 'card');
  const frame = node('FRAME');
  const inst = node('INSTANCE', { detachInstance: () => frame });
  const r = runtime(node('PAGE', { children: [src, inst, frame] }));
  const info = await r.handlers.clone({ id: src.id, x: 100 });
  const copy = src.parent.children.find(n => n.id === info.id);
  assert.equal(info.idMap[leaf.id], copy.children[0].children[0].id);
  assert.equal(info.key, undefined);
  assert.equal((await r.handlers.find_by_key({ key: 'card' })).id, src.id);
  assert.equal((await r.handlers.detach({ id: inst.id })).id, frame.id);
});

test('delete checks every id first and refuses component masters without force', async () => {
  const comp = node('COMPONENT');
  const holder = node('FRAME', { children: [comp] });
  const plain = node('FRAME');
  const r = runtime(node('PAGE', { children: [plain, holder] }));
  await assert.rejects(r.handlers.delete({ ids: [plain.id, holder.id] }), /component master/);
  assert.equal(plain.removed, false);
  const del = await r.handlers.delete({ ids: [plain.id, '9:999999'] });
  assert.deepEqual([[...del.deleted], [...del.alreadyGone]], [[plain.id], ['9:999999']]);
});

test('query deep lists descendants and empty cursors start a new read', async () => {
  const inner = node('TEXT');
  const frame = node('FRAME', { children: [inner] });
  const page = node('PAGE', { children: [frame] });
  const r = runtime(page);
  assert.equal((await r.handlers.query({})).length, 1);
  assert.equal((await r.handlers.query({ deep: true })).length, 2);
  const read = await r.handlers.read_nodes({ id: frame.id, cursor: '' });
  assert.equal(read.nodes[0].id, frame.id);
});
