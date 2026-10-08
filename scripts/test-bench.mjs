import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { benchmarkTargets, summarize } from './bench-workloads.mjs';

async function fixture(t, exec) {
  const calls = [];
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://localhost');
      response.setHeader('Content-Type', 'application/json');
      if (url.pathname === '/health') return response.end(JSON.stringify({ stats: { memoryMb: 12 } }));
      if (url.pathname === '/benchmark') return response.end(JSON.stringify({ version: 'test', pluginRoundTrip: null }));
      let body = '';
      for await (const chunk of request) body += chunk;
      const call = { sessionId: url.searchParams.get('sessionId'), ...JSON.parse(body) };
      calls.push(call);
      response.end(JSON.stringify(await exec(call)));
    } catch (error) {
      response.statusCode = 500; response.end(JSON.stringify({ error: error.message }));
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); server.close(); });
  return { base: `http://127.0.0.1:${server.address().port}`, calls };
}

const result = data => ({ success: true, data });
function success(call) {
  if (call.operation === 'read_nodes') return result({ nodes: [{ id: call.params.cursor ? 'child' : call.params.id }],
    nextCursor: call.params.cursor ? null : `${call.sessionId}:next`, complete: !!call.params.cursor });
  if (call.operation === 'index_scan') return result({ complete: true, nodesTruncated: false });
  if (call.operation === 'export_image') return result({ base64: Buffer.from('png').toString('base64'), width: 100, height: 80 });
  throw new Error(`Unexpected operation: ${call.operation}`);
}

function cli(args) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ['scripts/bench.mjs', ...args]);
    let stdout = '', stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.on('error', reject);
    child.on('exit', code => resolve({ code, stdout, stderr }));
  });
}

test('live benchmark paginates reads, measures all workloads and runs tabs concurrently', { timeout: 5000 }, async t => {
  let active = 0, peak = 0;
  const { base, calls } = await fixture(t, async call => {
    active++; peak = Math.max(peak, active);
    await new Promise(resolve => setTimeout(resolve, 5));
    active--;
    return success(call);
  });
  const report = await benchmarkTargets(base, [{ sessionId: 'one', nodeId: 'frame:1' }, { sessionId: 'two', nodeId: 'frame:2' }], 2);
  assert.equal(report.concurrentTabs, 2);
  assert.ok(peak >= 2);
  assert.equal(report.serverRssBeforeMb, 12);
  for (const target of report.targets) {
    assert.equal(target.workloads.readFrame.n, 2);
    assert.equal(target.workloads.readFrame.samples[0].nodes, 2);
    assert.equal(target.workloads.readFrame.samples[0].pages, 2);
    assert.equal(target.workloads.readFrame.samples[0].complete, true);
    assert.ok(target.workloads.readFrame.samples[0].responseBytes > 0);
    assert.equal(target.workloads.indexFrame.samples[0].complete, true);
    assert.equal(target.workloads.exportPng.samples[0].imageBytes, 3);
  }
  assert.equal(calls.length, 16);
  assert.ok(calls.filter(call => call.params.cursor).every(call => call.params.cursor.startsWith(call.sessionId)));
});

test('live benchmark exposes bridge failures and stops loading the failed tab', async t => {
  const { base, calls } = await fixture(t, () => ({ success: false, error: 'Plugin disconnected' }));
  const report = await benchmarkTargets(base, [{ sessionId: 'one', nodeId: 'frame' }]);
  assert.equal(report.targets[0].workloads.readFrame.error, 'Plugin disconnected');
  assert.equal(calls.length, 1);
  await assert.rejects(benchmarkTargets(base, [{ sessionId: 'one', nodeId: 'a' }, { sessionId: 'one', nodeId: 'b' }]), /one target per session/);
  await assert.rejects(benchmarkTargets(base, [{ sessionId: 'one', nodeId: 'a' }], 0), /samples/);
});

test('benchmark rejects repeated cursors and CLI preserves failure status in JSON', async t => {
  const { base } = await fixture(t, () => result({ nodes: [], nextCursor: 'same', complete: false }));
  const output = await cli(['--url', base, '--target', 'one=frame', '--samples', '1']);
  assert.equal(output.code, 1, output.stderr);
  const report = JSON.parse(output.stdout);
  assert.match(report.workloads.targets[0].workloads.readFrame.error, /repeated a cursor/);
  const invalid = await cli(['--target', 'invalid']);
  assert.equal(invalid.code, 1);
  assert.match(invalid.stderr, /SESSION=NODE/);
});

test('benchmark reports sorted percentile samples', () => {
  const report = summarize([5, 1, 3, 2, 4].map(durationMs => ({ durationMs })));
  assert.equal(report.p50Ms, 3);
  assert.equal(report.p95Ms, 5);
});
