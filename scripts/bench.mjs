// Cold: node scripts/bench.mjs [binary]
// Live: node scripts/bench.mjs --url http://127.0.0.1:41730 --target SESSION=NODE [--target SESSION=NODE] [--samples 3]
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { statSync } from 'node:fs';
import { benchmarkTargets, requestJson } from './bench-workloads.mjs';

const args = process.argv.slice(2), targets = [];
let base, samples = 3, bin = `target/release/figma-rust-mcp${process.platform === 'win32' ? '.exe' : ''}`;
for (let i = 0; i < args.length; i++) {
  const arg = args[i];
  if (arg === '--url') base = args[++i];
  else if (arg === '--samples') samples = Number(args[++i]);
  else if (arg === '--target') {
    const target = args[++i] || '', separator = target.indexOf('=');
    if (separator < 1 || separator === target.length - 1) throw new Error('--target must be SESSION=NODE');
    targets.push({ sessionId: target.slice(0, separator), nodeId: target.slice(separator + 1) });
  } else if (!arg.startsWith('-') && i === 0) bin = arg;
  else throw new Error(`Unknown argument: ${arg}`);
}
if (args.includes('--url') && !base) throw new Error('--url needs a server URL');
if (!Number.isInteger(samples) || samples < 1 || samples > 10) throw new Error('--samples must be from 1 to 10');
if (targets.length && !base) throw new Error('--target requires --url for an existing Figma connection');
if (base && !['http:', 'https:'].includes(new URL(base).protocol)) throw new Error('--url must use HTTP or HTTPS');
let child, coldStartMs;
try {
  if (!base) {
    const reservation = createServer();
    await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
    const port = reservation.address().port;
    await new Promise(resolve => reservation.close(resolve));
    base = `http://127.0.0.1:${port}`;
    const start = performance.now();
    let spawnError;
    child = spawn(bin, ['--server', '--port', String(port)], { stdio: 'ignore' });
    child.on('error', error => { spawnError = error; });
    while (coldStartMs === undefined) {
      if (spawnError) throw spawnError;
      if (child.exitCode !== null) throw new Error(`Server exited: ${child.exitCode}`);
      if (performance.now() - start > 10000) throw new Error('Server did not answer /health within 10s');
      try {
        const response = await fetch(`${base}/health`, { signal: AbortSignal.timeout(500) });
        if (response.ok) coldStartMs = performance.now() - start;
      } catch {}
      if (coldStartMs === undefined) await new Promise(resolve => setTimeout(resolve, 2));
    }
  }
  const report = await requestJson(base, '/benchmark', {});
  report.mode = child ? 'cold-start' : 'live';
  if (child) {
    report.coldStartMs = coldStartMs;
    report.binarySizeMb = statSync(bin).size / 1024 / 1024;
  }
  if (targets.length) report.workloads = await benchmarkTargets(base, targets, samples);
  console.log(JSON.stringify(report, null, 2));
  if (report.workloads?.targets.some(target => Object.values(target.workloads).some(workload => workload.error || workload.samples?.some(sample => sample.complete === false)))) process.exitCode = 1;
} finally {
  child?.kill();
}
