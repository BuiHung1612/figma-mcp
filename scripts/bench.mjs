// Benchmark: cold start measured from outside (spawn → /health answers), then the
// server's in-process report from POST /benchmark (same numbers as the plugin "Bench" tab).
// Usage: npm run bench   or   node scripts/bench.mjs [path/to/figma-rust-mcp]
import { spawn } from "node:child_process";
import { statSync } from "node:fs";

const bin = process.argv[2] ?? `target/release/figma-rust-mcp${process.platform === "win32" ? ".exe" : ""}`;
const port = 40000 + Math.floor(Math.random() * 10000);
const base = `http://127.0.0.1:${port}`;

const t0 = performance.now();
const child = spawn(bin, ["--server", "--port", String(port)], { stdio: "ignore" });
let coldStartMs;
try {
  while (coldStartMs === undefined) {
    if (performance.now() - t0 > 10_000) throw new Error("server did not answer /health within 10s");
    try {
      if ((await fetch(`${base}/health`)).ok) coldStartMs = performance.now() - t0;
    } catch {
      await new Promise((r) => setTimeout(r, 2));
    }
  }
  const r = await (await fetch(`${base}/benchmark`, { method: "POST" })).json();
  const lat = (s) => (s ? `p50 ${s.p50Ms.toFixed(3)}ms · p95 ${s.p95Ms.toFixed(3)}ms (n=${s.n})` : "skipped");
  const rows = {
    version: `v${r.version}`,
    "binary size": `${(statSync(bin).size / 1024 / 1024).toFixed(1)}MB`,
    "cold start (spawn → /health)": `${coldStartMs.toFixed(1)}ms`,
    "startup (main → listening)": r.startupMs != null ? `${r.startupMs.toFixed(2)}ms` : "—",
    "RAM (RSS)": r.memoryMb != null ? `${r.memoryMb.toFixed(1)}MB` : "n/a on this OS",
    "MCP tools/list (in-process)": lat(r.mcpToolsList),
    "plugin round trip": r.pluginRoundTrip ? lat(r.pluginRoundTrip) : "skipped (fresh server has no Figma session)",
  };
  for (const [k, v] of Object.entries(rows)) console.log(`${k.padEnd(30)} ${v}`);
} finally {
  child.kill();
}
