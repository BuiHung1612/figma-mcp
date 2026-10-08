import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { createInterface } from 'node:readline';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
async function until(predicate) {
  const deadline = Date.now() + 5000;
  while (!await predicate()) { if (Date.now() > deadline) throw new Error('Timed out'); await delay(10); }
}
const decode = result => JSON.parse(result.content[0].text);
const ok = result => { assert.notEqual(result.isError, true, result.content?.[0]?.text); return result; };

// Uses only Node built-ins (Node >=22 supplies WebSocket) and the actual Rust
// daemon. The canvas is simulated; this verifies transport/agent isolation.
test('multiple tabs, reconnect ownership and independent frame tasks across MCP clients', { timeout: 20000 }, async t => {
  assert.equal(typeof WebSocket, 'function', 'This integration check requires Node >=22');
  const reservation = createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const binary = process.env.FIGMA_TEST_BINARY || 'target/debug/figma-rust-mcp';
  const server = spawn(binary, ['--server', '--port', String(port)], { stdio: ['ignore', 'ignore', 'pipe'], env: { ...process.env, FIGMA_RUST_MCP_PORT: String(port) } });
  let stderr = ''; server.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-4000); });
  const processes = [server], sockets = [], latest = new Map(), actions = [];
  t.after(() => { for (const socket of sockets) socket.close(); for (const process of processes) process.kill(); });
  const base = `http://127.0.0.1:${port}`;
  await until(async () => { try { return (await fetch(`${base}/health`)).ok; } catch { assert.equal(server.exitCode, null, stderr); return false; } });
  let rpcId = 0;
  async function rpc(name, args) {
    const response = await fetch(`${base}/mcp`, { method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: ++rpcId, method: 'tools/call', params: { name, arguments: args } }) });
    return (await response.json()).result;
  }
  function agent() {
    const process = spawn(binary, ['--stdio', '--port', String(port)], { env: { ...globalThis.process.env, FIGMA_RUST_MCP_PORT: String(port) } });
    processes.push(process); process.stderr.resume();
    let id = 0;
    const pending = new Map();
    createInterface({ input: process.stdout }).on('line', line => {
      const response = JSON.parse(line); const resolve = pending.get(response.id);
      if (resolve) { pending.delete(response.id); resolve(response.result); }
    });
    return (name, args) => new Promise(resolve => {
      pending.set(++id, resolve);
      process.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method: 'tools/call', params: { name, arguments: args } }) + '\n');
    });
  }
  const paused = deferred(), resume = deferred();
  async function plugin(sid, existing) {
    const state = existing || {};
    if (!state.runtime) {
      const page = { id: 'page', type: 'PAGE' };
      const nodes = new Map();
      for (const suffix of ['a', 'b', 'c']) {
        const root = { id: `frame-${suffix}`, name: suffix, type: 'FRAME', parent: page, children: [] };
        const child = { id: `child-${suffix}`, type: 'TEXT', parent: root }; root.children.push(child);
        nodes.set(root.id, root); nodes.set(child.id, child);
      }
      state.runtime = vm.createContext({ handlers: {}, findNodeByIdAsync: async id => nodes.get(id), yieldToUI: async () => {} });
      vm.runInContext(readFileSync('plugin-src/task-scope.js', 'utf8'), state.runtime);
      state.runtime.handlers.get_design = async params => ({ id: params.id, type: 'FRAME', name: params.id, tab: sid });
      state.runtime.handlers.read_nodes = async params => ({ schemaVersion: 4, pageId: 'page', revision: 0, scope: { id: params.id }, nodes: [{ id: params.id, type: 'FRAME', name: params.id, tab: sid }], nextCursor: null, complete: true });
      state.runtime.handlers.status = async () => ({ tab: sid });
      state.runtime.handlers.modify = async params => {
        actions.push(params.note);
        if (params.note === 'paused') { paused.resolve(); await resume.promise; }
        return { id: params.id, note: params.note };
      };
      state.tail = Promise.resolve();
    }
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws?sessionId=${sid}&fileName=SameFile&documentId=same-document`);
    sockets.push(socket); state.socket = socket; latest.set(sid, state);
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (message.type === 'server-hello') {
        for (const id of state.runtime.frameTasks.keys()) if (!message.activeTaskIds.includes(id)) state.runtime.frameTasks.delete(id);
        return;
      }
      if (!message.operation) return;
      if (state.ignoreNext) { state.ignoreNext = false; state.ignored.resolve(message); return; }
      socket.send(JSON.stringify({ type: 'ack', id: message.id }));
      state.tail = state.tail.then(async () => {
        try {
          const checked = await state.runtime.validateTaskOperation(message.operation, message.params);
          const data = await state.runtime.handlers[checked.operation](checked.params);
          latest.get(sid).socket.send(JSON.stringify({ id: message.id, sessionId: sid, success: true, data }));
        } catch (error) { latest.get(sid).socket.send(JSON.stringify({ id: message.id, sessionId: sid, success: false, error: error.message })); }
      });
    });
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    socket.send(JSON.stringify({ type: 'runtime-capabilities', runtimeVersion: 'test', protocolVersion: 3, documentId: 'same-document', operations: Object.keys(state.runtime.handlers) }));
    return state;
  }
  const tabA = await plugin('tab-a'), tabB = await plugin('tab-b');
  const first = agent(), second = agent();
  const scopedSearch = decode(ok(await rpc('figma_read', { operation: 'search_nodes', query: 'child', sessionId: 'tab-a' })));
  assert.equal(scopedSearch.complete, false);
  assert.deepEqual(scopedSearch.nodes, []);

  const ambiguous = await rpc('figma_read', { operation: 'get_design', nodeId: 'frame-a' });
  assert.equal(ambiguous.isError, true); assert.match(ambiguous.content[0].text, /Ambiguous/);
  const missing = await rpc('figma_read', { operation: 'get_design', sessionId: 'missing' });
  assert.equal(missing.isError, true);
  const taskA = decode(ok(await first('figma_task', { action: 'start', sessionId: 'tab-a', frameId: 'frame-a' })));
  const collision = await second('figma_task', { action: 'start', sessionId: 'tab-b', frameId: 'frame-a' });
  assert.equal(collision.isError, true); assert.match(collision.content[0].text, /reserved/);
  const taskB = decode(ok(await second('figma_task', { action: 'start', sessionId: 'tab-a', frameId: 'frame-b' })));
  const taskC = decode(ok(await rpc('figma_task', { action: 'start', sessionId: 'tab-b', frameId: 'frame-c' })));
  assert.equal(decode(ok(await first('figma_get_selection', { taskId: taskA.taskId }))).id, 'frame-a');
  assert.equal(decode(ok(await second('figma_get_selection', { taskId: taskB.taskId }))).id, 'frame-b');
  const writeA = first('figma_write', { taskId: taskA.taskId, code: 'await figma.modify({id:"child-a",note:"paused"}); await figma.modify({id:"child-a",note:"second"});' });
  await paused.promise;
  const writeB = second('figma_write', { taskId: taskB.taskId, code: 'await figma.modify({id:"child-b",note:"other-task"});' });
  ok(await rpc('figma_write', { taskId: taskC.taskId, code: 'await figma.modify({id:"child-c",note:"other-tab"});' }));
  assert.deepEqual(actions, ['paused', 'other-tab']);
  resume.resolve(); ok(await writeA); ok(await writeB);
  assert.deepEqual(actions, ['paused', 'other-tab', 'second', 'other-task']);
  const returned = ok(await first('figma_write', { taskId: taskA.taskId,
    code: 'await figma.modify({id:"child-a",note:"settled"}); return "done";' }));
  assert.equal(returned.content[0].text, 'Result: "done"');
  const rejected = await first('figma_write', { taskId: taskA.taskId,
    code: 'await figma.modify({id:"child-a",note:"before-error"}); throw new Error("after-await");' });
  assert.equal(rejected.isError, true); assert.match(rejected.content[0].text, /after-await/);
  const outside = await first('figma_write', { taskId: taskA.taskId, code: 'await figma.modify({id:"child-b",note:"escape"});' });
  assert.equal(outside.isError, true); assert.ok(!actions.includes('escape'));
  const rawWrite = await fetch(`${base}/exec?sessionId=tab-b`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ operation: 'modify', params: { id: 'child-a' } }) });
  assert.equal((await rawWrite.json()).success, false);

  // A replacement socket recovers an op that the old socket never ACKed.
  tabA.ignoreNext = true; tabA.ignored = deferred(); const oldSocket = tabA.socket;
  const recovering = rpc('figma_read', { operation: 'get_design', nodeId: 'frame-a', sessionId: 'tab-a' });
  const ignored = await tabA.ignored.promise;
  await plugin('tab-a', tabA); oldSocket.close();
  assert.equal(decode(ok(await recovering)).nodes[0].tab, 'tab-a');
  await delay(30);
  assert.equal(decode(ok(await rpc('figma_read', { operation: 'get_design', nodeId: 'frame-b', sessionId: 'tab-a' }))).nodes[0].id, 'frame-b');
  // Cross-tab response spoofing cannot consume another tab's pending request.
  tabA.ignoreNext = true; tabA.ignored = deferred();
  const routed = rpc('figma_read', { operation: 'get_design', nodeId: 'frame-a', sessionId: 'tab-a' });
  const waiting = await tabA.ignored.promise;
  tabB.socket.send(JSON.stringify({ id: waiting.id, success: true, data: { hijacked: true } }));
  const wrongHttp = await fetch(`${base}/response`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ id: waiting.id, sessionId: 'tab-b', success: true, data: { hijacked: true } }) });
  assert.equal((await wrongHttp.json()).ok, false);
  tabA.socket.send(JSON.stringify({ id: waiting.id, sessionId: 'tab-a', success: true, data: { id: 'frame-a', tab: 'tab-a' } }));
  assert.equal(decode(ok(await routed)).tab, 'tab-a');
  assert.ok(ignored.id);
  ok(await first('figma_task', { action: 'end', taskId: taskA.taskId }));
  ok(await second('figma_task', { action: 'end', taskId: taskB.taskId }));
  tabB.socket.close();
  await until(async () => !(await (await fetch(`${base}/sessions`)).json()).sessions.find(session => session.id === 'tab-b').connected);
  assert.equal(decode(ok(await rpc('figma_task', { action: 'end', taskId: taskC.taskId }))).pluginDisconnected, true);
  await plugin('tab-b', tabB);
  const restarted = decode(ok(await rpc('figma_task', { action: 'start', sessionId: 'tab-b', frameId: 'frame-c' })));
  ok(await rpc('figma_task', { action: 'end', taskId: restarted.taskId }));
});

test('index sync travels from plugin through UI to bridge and returns a matching confirmation', { timeout: 20000 }, async t => {
  const { indexSyncUi } = await import('./helpers/index-sync-ui.mjs');
  const reservation = createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const server = spawn(process.env.FIGMA_TEST_BINARY || 'target/debug/figma-rust-mcp',
    ['--server', '--port', String(port)], { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = ''; server.stderr.on('data', bytes => { stderr = (stderr + bytes).slice(-4000); });
  const sockets = [];
  t.after(() => { sockets.forEach(socket => socket.close()); server.kill(); });
  const base = `http://127.0.0.1:${port}`;
  await until(async () => { try { return (await fetch(`${base}/health`)).ok; } catch { assert.equal(server.exitCode, null, stderr); return false; } });
  const ui = indexSyncUi(null);
  const page = { id: 'page', children: [{ id: 'frame', name: 'Frame', type: 'FRAME', width: 100, height: 100 }] };
  const runtime = vm.createContext({ figma: { currentPage: page, root: { name: 'Sync test' },
    ui: { postMessage: msg => ui.handleIndexMessage(msg) } }, handlers: {}, console, setTimeout, Map, Set, Uint8Array });
  for (const file of ['utils', 'token-helpers', 'read-helpers', 'handlers-read-detail', 'handlers-read', 'handlers-tokens', 'handlers-write-ops', 'node-reader']) {
    vm.runInContext(readFileSync(`plugin-src/${file}.js`, 'utf8'), runtime);
  }
  runtime.handlers.get_styles = async () => ({});
  runtime.handlers.get_variables = async () => ({});
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  Object.assign(ui, { WebSocket, BRIDGE: base, sessionId: 'sync-test', fileName: 'Sync test', documentId: 'sync-doc',
    uiDisposed: false, wsConnected: false, everConnected: false, retryTimer: null, polling: false, consecutiveErrors: 0,
    currentPort: port, window: {}, READ_OPS: [], setInterval: () => 1, clearInterval() {},
    sendRuntimeCapabilities() {}, setStatus() {}, updateRuntimeVersion() {}, startLongPoll() {},
    dispatchToMain() {}, showSetupGuide() {}, parent: { postMessage: () => runtime.handlers.index_scan({ deferComponents: true }) } });
  vm.runInContext(html.slice(html.indexOf('    function connectWs()'), html.indexOf('    async function startLongPoll()')), ui);
  const connect = async () => { ui.connectWs(); sockets.push(ui.ws); await until(() => ui.wsConnected); };
  await connect();
  const confirmed = ui.receiveIndexAck;
  const acks = [];
  ui.receiveIndexAck = ack => acks.push(ack);
  await runtime.handlers.index_scan({ deferComponents: true });
  await until(() => acks.length === 1);
  assert.equal(acks[0].success, true);
  assert.equal(ui.reindexBtn.disabled, true, 'local scan completion must wait for bridge ack');
  confirmed({ ...acks[0], scanId: 'older-scan' });
  assert.equal(ui.reindexBtn.disabled, true);
  confirmed(acks.shift());
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.reindexBtn.title, /Last synced/);
  const lastSuccess = ui.reindexBtn.title;
  ui.receiveIndexAck = ack => { acks.push(ack); confirmed(ack); };
  const unmatched = { type: 'index-start', pageId: 'page', scanId: 'unmatched', revision: 0, scope: { id: 'page' } };
  // The UI begins a scan but the bridge never receives its start/chunks.
  ui.handleIndexMessage({ ...unmatched, type: 'index-progress', stage: 'starting' });
  ui.handleIndexMessage({ ...unmatched, type: 'index-update', data: {} });
  await until(() => acks.length === 1);
  assert.equal(acks[0].success, false);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /no longer active/);
  assert.equal(ui.reindexBtn.title, lastSuccess);

  ui.handleIndexMessage({ ...unmatched, scanId: 'disconnect' });
  ui.ws.close();
  await until(() => !ui.wsConnected);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /disconnected/);
  await connect();
  acks.length = 0;
  await ui.triggerManualReindex();
  await until(() => acks.length === 1);
  assert.equal(acks[0].success, true);
  assert.equal(ui.reindexBtn.disabled, false);

  runtime.handlers.get_styles = async () => { throw new Error('Styles unavailable'); };
  await assert.rejects(runtime.handlers.index_scan({ deferComponents: true }), /Styles unavailable/);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /Styles unavailable/);
  runtime.handlers.get_styles = async () => { runtime.figma.currentPage = { id: 'new-page', children: [] }; return {}; };
  const cancelled = await runtime.handlers.index_scan({ deferComponents: true });
  assert.equal(cancelled.cancelled, true);
  assert.equal(ui.reindexBtn.disabled, false);
  assert.match(ui.indexProgressText.textContent, /Document changed/);
  runtime.handlers.get_styles = async () => ({});
  acks.length = 0;
  await runtime.handlers.index_scan({ deferComponents: true });
  await until(() => acks.length === 1);
  assert.equal(acks[0].pageId, 'new-page');
  assert.equal(acks[0].success, true);
  assert.equal(ui.reindexBtn.disabled, false);

  const { benchmarkTargets } = await import('./bench-workloads.mjs');
  page.type = 'PAGE';
  const frame = page.children[0];
  frame.parent = page;
  frame.children = Array.from({ length: 600 }, (_, i) => ({ id: `bench:${i}`, type: 'TEXT',
    name: `Child ${i}`, characters: `Text ${i}`, parent: frame, width: 10, height: 10 }));
  let exported = 0;
  frame.exportAsync = async () => { exported++; return new Uint8Array([1, 2, 3]); };
  runtime.figma.currentPage = page;
  runtime.figma.getNodeByIdAsync = async id => id === frame.id ? frame : null;
  const operations = [];
  ui.READ_OPS = ['read_nodes', 'index_scan', 'export_image'];
  ui.dispatchToMain = async req => {
    if (!req.operation) return;
    operations.push(req.operation);
    try {
      const data = await runtime.handlers[req.operation](req.params);
      ui.ws.send(JSON.stringify({ id: req.id, sessionId: ui.sessionId, success: true, data }));
    } catch (error) {
      ui.ws.send(JSON.stringify({ id: req.id, sessionId: ui.sessionId, success: false, error: error.message }));
    }
  };
  const measurements = await benchmarkTargets(base, [{ sessionId: ui.sessionId, nodeId: frame.id }], 1);
  const workloads = measurements.targets[0].workloads;
  assert.equal(workloads.readFrame.samples[0].nodes, 601);
  assert.equal(workloads.readFrame.samples[0].pages, 2);
  assert.equal(workloads.readFrame.samples[0].complete, true);
  assert.equal(workloads.indexFrame.samples[0].complete, true);
  assert.equal(workloads.exportPng.samples[0].imageBytes, 3);
  assert.equal(exported, 1);
  assert.deepEqual(operations, ['read_nodes', 'read_nodes', 'index_scan', 'export_image']);

});
