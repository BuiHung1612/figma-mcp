// Read-only live workloads. The server and Figma plugin must already be running.
export async function requestJson(base, route, body) {
  const response = await fetch(new URL(route, base), {
    method: body === undefined ? 'GET' : 'POST',
    headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(120000),
  });
  if (!response.ok) throw new Error(`${route}: HTTP ${response.status}`);
  return response.json();
}

export function summarize(samples) {
  const times = samples.map(sample => sample.durationMs).sort((a, b) => a - b);
  const pick = q => times[Math.round((times.length - 1) * q)];
  return { n: times.length, p50Ms: pick(.5), p95Ms: pick(.95), maxMs: times.at(-1), samples };
}

export async function benchmarkTargets(base, targets, samples = 3) {
  if (!Number.isInteger(samples) || samples < 1 || samples > 10) throw new Error('samples must be from 1 to 10');
  if (!targets.length || targets.some(target => !target.sessionId || !target.nodeId)) throw new Error('Each target needs sessionId and nodeId');
  if (new Set(targets.map(target => target.sessionId)).size !== targets.length) throw new Error('Use one target per session for concurrent tab measurements');
  const memoryBefore = (await requestJson(base, '/health')).stats?.memoryMb ?? null;
  const started = performance.now();
  const results = await Promise.all(targets.map(async ({ sessionId, nodeId }) => {
    const workloads = {};
    const exec = async (operation, params) => {
      const result = await requestJson(base, `/exec?sessionId=${encodeURIComponent(sessionId)}`, { operation, params });
      if (!result.success) throw new Error(result.error || `${operation} failed`);
      return result.data;
    };
    const runs = {
      readFrame: async () => {
        let cursor, nodes = 0, pages = 0, responseBytes = 0, complete;
        const cursors = new Set();
        do {
          const data = await exec('read_nodes', cursor ? { cursor, limit: 500 } :
            { id: nodeId, depth: 'full', expandInstances: true, fields: ['geometry', 'content', 'style', 'text'], limit: 500 });
          if (!Array.isArray(data.nodes)) throw new Error('read_nodes returned no node list');
          nodes += data.nodes.length; pages++;
          responseBytes += Buffer.byteLength(JSON.stringify(data));
          complete = data.complete === true;
          cursor = data.nextCursor;
          if (cursor && cursors.has(cursor)) throw new Error('read_nodes repeated a cursor');
          if (cursor) cursors.add(cursor);
          if (cursor && pages >= 100) throw new Error('read_nodes exceeded 100 pages');
        } while (cursor);
        return { nodes, pages, responseBytes, complete };
      },
      indexFrame: async () => {
        const data = await exec('index_scan', { id: nodeId, depth: 'full', expandInstances: true, deferComponents: true });
        if (data.cancelled) throw new Error('Index cancelled by a document or page change');
        return { complete: data.complete === true, nodesTruncated: data.nodesTruncated === true };
      },
      exportPng: async () => {
        const data = await exec('export_image', { id: nodeId, format: 'PNG', scale: 1 });
        if (typeof data.base64 !== 'string') throw new Error('export_image returned no image');
        return { imageBytes: Buffer.from(data.base64, 'base64').length, width: data.width, height: data.height };
      },
    };
    for (const [name, run] of Object.entries(runs)) {
      const timings = [];
      try {
        for (let i = 0; i < samples; i++) {
          const start = performance.now();
          const data = await run();
          timings.push({ durationMs: performance.now() - start, ...data });
        }
        workloads[name] = summarize(timings);
      } catch (error) {
        workloads[name] = { error: error.message, completedSamples: timings.length, samples: timings };
        break; // Do not keep loading a tab after a timeout or disconnect.
      }
    }
    return { sessionId, nodeId, workloads };
  }));
  const memoryAfter = (await requestJson(base, '/health')).stats?.memoryMb ?? null;
  return {
    mode: 'live', concurrentTabs: targets.length, samplesPerWorkload: samples,
    wallTimeMs: performance.now() - started,
    serverRssBeforeMb: memoryBefore, serverRssAfterMb: memoryAfter,
    note: 'RSS is server memory at the boundaries, not peak memory or Figma memory. Reads may use the normal cache. Index timing includes plugin scan/response, not bridge commit acknowledgement.',
    targets: results,
  };
}
