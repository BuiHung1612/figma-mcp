#!/usr/bin/env node

/**
 * Figma Rust MCP - NPX Binary Runner
 * Automatically detects platform/arch, downloads prebuilt Rust binary from GitHub Releases,
 * caches it locally, and executes it transparently.
 */

import { spawn, execSync, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import https from 'node:https';
import http from 'node:http';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline/promises';
import { configureAgent, detectAgents } from './agent-config.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Read package.json version
const pkgPath = path.resolve(__dirname, '../package.json');
let pkgVersion = null;
let repoOwner = 'BuiHung1612';
let repoName = 'figma-rust-mcp';

try {
  if (fs.existsSync(pkgPath)) {
    const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
    if (pkg.version) pkgVersion = pkg.version;
    if (pkg.repository && typeof pkg.repository.url === 'string') {
      const m = pkg.repository.url.match(/github\.com[/:]([^/]+)\/([^/.]+)/);
      if (m) {
        repoOwner = m[1];
        repoName = m[2];
      }
    }
  }
} catch (_) {}
if (!pkgVersion) throw new Error(`Cannot read package version from ${pkgPath}`);

// ─── TERMINAL UI ──────────────────────────────────────────────────────────────

const useColor = process.stdout.isTTY && !process.env.NO_COLOR;
const paint = code => text => useColor ? `\x1b[${code}m${text}\x1b[0m` : String(text);
const bold = paint('1'), dim = paint('2'), green = paint('32'), yellow = paint('33'), red = paint('31'), cyan = paint('36');
const tag = code => process.stderr.isTTY && !process.env.NO_COLOR ? `\x1b[${code}m[figma-rust-mcp]\x1b[0m` : '[figma-rust-mcp]';
const tilde = p => p.startsWith(os.homedir()) ? `~${p.slice(os.homedir().length)}` : p;
const ok = (msg, detail = '') => console.log(`  ${green('✓')} ${msg}${detail ? `  ${dim(detail)}` : ''}`);
const note = msg => console.log(`    ${dim(msg)}`);
const step = (n, total, title) => console.log(`\n${cyan(`[${n}/${total}]`)} ${bold(title)}`);

function getPlatformArch() {
  const platform = process.platform;
  const arch = process.arch;

  let osName = '';
  let archName = '';
  let ext = 'tar.gz';
  let binName = 'figma-rust-mcp';

  if (platform === 'darwin') {
    osName = 'macos';
    archName = arch === 'arm64' ? 'arm64' : 'x86_64';
  } else if (platform === 'linux') {
    osName = 'linux';
    archName = arch === 'arm64' ? 'arm64' : 'x86_64';
  } else if (platform === 'win32') {
    osName = 'windows';
    archName = 'x86_64';
    ext = 'zip';
    binName = 'figma-rust-mcp.exe';
  } else {
    throw new Error(`Unsupported operating system: ${platform}`);
  }

  const assetName = `figma-rust-mcp-${osName}-${archName}.${ext}`;
  return { osName, archName, ext, binName, assetName };
}

function getCacheDir(version) {
  const platform = process.platform;
  let baseCache = '';

  if (platform === 'win32') {
    baseCache = process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local');
  } else if (platform === 'darwin') {
    baseCache = path.join(os.homedir(), 'Library', 'Caches');
  } else {
    baseCache = process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache');
  }

  return path.join(baseCache, 'figma-rust-mcp', `v${version}`);
}

// Progress bar on stderr (stdout carries MCP stdio traffic); silent when stderr is not a terminal.
function downloadProgress(total) {
  const tty = process.stderr.isTTY;
  const started = Date.now();
  let received = 0, lastDraw = 0;
  const mb = bytes => (bytes / 1048576).toFixed(1);
  const draw = () => {
    const speed = `${mb(received / Math.max((Date.now() - started) / 1000, 0.001))} MB/s`;
    if (!total) return process.stderr.write(`\r\x1b[2K  ↓ ${mb(received)} MB  ${speed}`);
    const ratio = Math.min(received / total, 1), width = 24, filled = Math.round(ratio * width);
    process.stderr.write(`\r\x1b[2K  ↓ \x1b[36m${'█'.repeat(filled)}\x1b[2m${'░'.repeat(width - filled)}\x1b[0m ${String(Math.round(ratio * 100)).padStart(3)}%  ${mb(received)}/${mb(total)} MB  ${speed}`);
  };
  return {
    add(bytes) {
      received += bytes;
      if (tty && Date.now() - lastDraw > 80) { lastDraw = Date.now(); draw(); }
    },
    done() { if (tty) { draw(); process.stderr.write('\n'); } },
  };
}

function downloadFile(url, destPath) {
  return new Promise((resolve, reject) => {
    const followRedirects = (currentUrl, redirectCount = 0) => {
      if (redirectCount > 10) {
        reject(new Error('Too many redirects while downloading binary'));
        return;
      }

      const client = currentUrl.startsWith('https') ? https : http;
      client.get(currentUrl, { headers: { 'User-Agent': 'figma-rust-mcp-npx' } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          followRedirects(res.headers.location, redirectCount + 1);
          return;
        }

        if (res.statusCode !== 200) {
          reject(new Error(`Failed to download binary: HTTP ${res.statusCode} from ${currentUrl}`));
          return;
        }

        const fileStream = fs.createWriteStream(destPath);
        const progress = downloadProgress(Number(res.headers['content-length']) || 0);
        res.on('data', chunk => progress.add(chunk.length));
        res.pipe(fileStream);

        fileStream.on('finish', () => {
          progress.done();
          fileStream.close(() => resolve());
        });

        fileStream.on('error', (err) => {
          fs.unlink(destPath, () => {});
          reject(err);
        });
      }).on('error', (err) => {
        reject(err);
      });
    };

    followRedirects(url);
  });
}

function extractArchive(archivePath, targetDir, ext) {
  fs.mkdirSync(targetDir, { recursive: true });

  if (ext === 'zip') {
    try {
      execSync(`tar -xf "${archivePath}" -C "${targetDir}"`, { stdio: 'pipe' });
      return;
    } catch (_) {
      execSync(`powershell -Command "Expand-Archive -Path '${archivePath}' -DestinationPath '${targetDir}' -Force"`, { stdio: 'pipe' });
      return;
    }
  }

  execSync(`tar -xzf "${archivePath}" -C "${targetDir}"`, { stdio: 'pipe' });
}

async function findOrDownloadBinary() {
  if (process.env.FIGMA_RUST_MCP_BIN && fs.existsSync(process.env.FIGMA_RUST_MCP_BIN)) {
    return process.env.FIGMA_RUST_MCP_BIN;
  }

  const localRelease = path.resolve(__dirname, '../target/release/figma-rust-mcp' + (process.platform === 'win32' ? '.exe' : ''));
  const localDebug = path.resolve(__dirname, '../target/debug/figma-rust-mcp' + (process.platform === 'win32' ? '.exe' : ''));

  // A stale local build would silently downgrade the service; only use one built from this version.
  const isCurrentBuild = bin => {
    try { return fs.existsSync(bin) && execFileSync(bin, ['--version'], { encoding: 'utf8', timeout: 5000 }).trim().endsWith(` ${pkgVersion}`); }
    catch (_) { return false; }
  };
  for (const bin of [localRelease, localDebug]) if (isCurrentBuild(bin)) return bin;

  const info = getPlatformArch();
  const cacheDir = getCacheDir(pkgVersion);
  const cachedBin = path.join(cacheDir, info.binName);

  if (fs.existsSync(cachedBin)) {
    if (process.platform !== 'win32') {
      try { fs.chmodSync(cachedBin, 0o755); } catch (_) {}
    }
    return cachedBin;
  }

  fs.mkdirSync(cacheDir, { recursive: true });
  const archivePath = path.join(cacheDir, info.assetName);

  const releaseUrl = `https://github.com/${repoOwner}/${repoName}/releases/download/v${pkgVersion}/${info.assetName}`;
  const latestUrl = `https://github.com/${repoOwner}/${repoName}/releases/latest/download/${info.assetName}`;

  process.stderr.write(`${tag(36)} Downloading binary for ${info.osName}-${info.archName} (v${pkgVersion})...\n`);

  try {
    await downloadFile(releaseUrl, archivePath);
  } catch (err) {
    process.stderr.write(`${tag(33)} Release v${pkgVersion} not found, trying latest release...\n`);
    try {
      await downloadFile(latestUrl, archivePath);
    } catch (fallbackErr) {
      throw new Error(`Failed to download figma-rust-mcp prebuilt binary:\n  - ${err.message}\n  - ${fallbackErr.message}\n\nPlease verify your internet connection or build from source: cargo build --release`);
    }
  }

  try {
    extractArchive(archivePath, cacheDir, info.ext);
  } finally {
    try { fs.unlinkSync(archivePath); } catch (_) {}
  }

  if (!fs.existsSync(cachedBin)) {
    const files = fs.readdirSync(cacheDir, { recursive: true });
    const found = files.find(f => path.basename(f) === info.binName);
    if (found) {
      const foundPath = path.join(cacheDir, found);
      fs.renameSync(foundPath, cachedBin);
    } else {
      throw new Error(`Binary ${info.binName} not found inside downloaded archive`);
    }
  }

  if (process.platform !== 'win32') {
    fs.chmodSync(cachedBin, 0o755);
  }

  process.stderr.write(`${tag(32)} Downloaded to ${tilde(cacheDir)}\n`);
  return cachedBin;
}

// ─── SERVICE MANAGEMENT ───────────────────────────────────────────────────────

/**
 * Install figma-rust-mcp as a background service that auto-starts on login.
 * - macOS: LaunchAgent plist (~/.config/figma-rust-mcp/launchd.plist → ~/Library/LaunchAgents/)
 * - Linux: systemd user service (~/.config/systemd/user/figma-rust-mcp.service)
 * - Windows: Task Scheduler XML via schtasks
 */
async function installService(binPath) {
  removeLegacyService();
  await stopRunningServer();
  const platform = process.platform;

  if (platform === 'darwin') {
    const plistLabel = 'io.github.figma-rust-mcp.server';
    const plistDir = path.join(os.homedir(), 'Library', 'LaunchAgents');
    const plistPath = path.join(plistDir, `${plistLabel}.plist`);

    fs.mkdirSync(plistDir, { recursive: true });

    const logDir = path.join(os.homedir(), 'Library', 'Logs', 'figma-rust-mcp');
    fs.mkdirSync(logDir, { recursive: true });

    const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${plistLabel}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${binPath}</string>
    <string>--server</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${path.join(logDir, 'stdout.log')}</string>
  <key>StandardErrorPath</key>
  <string>${path.join(logDir, 'stderr.log')}</string>
  <key>ThrottleInterval</key>
  <integer>5</integer>
</dict>
</plist>`;

    fs.writeFileSync(plistPath, plist, 'utf8');

    execSync(`launchctl load -w "${plistPath}"`, { stdio: 'pipe' });

    await serviceInstalled(`Logs ${tilde(path.join(logDir, 'stdout.log'))}`);

  } else if (platform === 'linux') {
    const serviceDir = path.join(os.homedir(), '.config', 'systemd', 'user');
    const servicePath = path.join(serviceDir, 'figma-rust-mcp.service');

    fs.mkdirSync(serviceDir, { recursive: true });

    const unit = `[Unit]
Description=Figma Rust MCP Bridge Server
After=network.target

[Service]
Type=simple
ExecStart=${binPath} --server
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
`;

    fs.writeFileSync(servicePath, unit, 'utf8');
    execSync(`systemctl --user daemon-reload`, { stdio: 'pipe' });
    execSync(`systemctl --user enable --now figma-rust-mcp`, { stdio: 'pipe' });

    await serviceInstalled('Logs: journalctl --user -u figma-rust-mcp');

  } else if (platform === 'win32') {
    const taskName = 'FigmaRustMCPServer';
    // Use schtasks to create a task that runs at login
    // The native launcher detaches the server and redirects output to a log file.
    const taskCommand = `"${binPath}" --background`;
    execFileSync('schtasks', ['/Create', '/F', '/TN', taskName,
      '/TR', taskCommand, '/SC', 'ONLOGON'], { stdio: 'pipe', windowsHide: true });

    // Start it immediately too
    try { execSync(`schtasks /Run /TN "${taskName}"`, { stdio: 'pipe' }); } catch (_) {}

    await serviceInstalled(`Task Scheduler: ${taskName}`);

  } else {
    console.error(`\x1b[31mUnsupported platform: ${platform}\x1b[0m`);
    process.exit(1);
  }
}

async function serviceInstalled(where) {
  const port = Number(process.env.FIGMA_RUST_MCP_PORT) || 41730;
  const deadline = Date.now() + 10000;
  while (!(await isOldServerUp(port)) && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 300));
  if (await isOldServerUp(port)) ok('Background service running', `http://127.0.0.1:${port}`);
  else console.log(`  ${yellow('!')} Service installed but not answering on port ${port} yet`);
  note(`Starts on login · ${where}`);
}

function isOldServerUp(port) {
  return new Promise(resolve => {
    const req = http.get({ host: '127.0.0.1', port, path: '/plugin/version', timeout: 500 }, res => {
      let body = '';
      res.on('data', chunk => { body += chunk; });
      res.on('end', () => { try { resolve(JSON.parse(body).name === 'figma-rust-mcp'); } catch (_) { resolve(false); } });
    });
    req.on('error', () => resolve(false));
    req.on('timeout', () => { req.destroy(); resolve(false); });
  });
}

/**
 * Stop the installed service and any other running server before installing the new one.
 * A still-running old version keeps port 41730, so the new one would move to 41731 while
 * agents stay pointed at the old server.
 */
async function stopRunningServer() {
  const quiet = cmd => { try { execSync(cmd, { stdio: 'pipe', windowsHide: true }); } catch (_) {} };
  if (process.platform === 'darwin') {
    quiet(`launchctl unload "${path.join(os.homedir(), 'Library', 'LaunchAgents', 'io.github.figma-rust-mcp.server.plist')}"`);
    quiet('pkill -x figma-rust-mcp');
  } else if (process.platform === 'linux') {
    quiet('systemctl --user stop figma-rust-mcp');
    quiet('pkill -x figma-rust-mcp');
  } else if (process.platform === 'win32') {
    quiet('schtasks /End /TN "FigmaRustMCPServer"');
    quiet('taskkill /F /T /IM figma-rust-mcp.exe');
  }
  const port = Number(process.env.FIGMA_RUST_MCP_PORT) || 41730;
  const deadline = Date.now() + 10000;
  while (await isOldServerUp(port)) {
    if (Date.now() > deadline) {
      console.log(`  ${yellow('!')} An old figma-rust-mcp still answers on port ${port}; stop it, then rerun setup.`);
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 300));
  }
}

/**
 * Uninstall the background service.
 */
async function uninstallService() {
  const platform = process.platform;

  if (platform === 'darwin') {
    const plistLabel = 'io.github.figma-rust-mcp.server';
    const plistPath = path.join(os.homedir(), 'Library', 'LaunchAgents', `${plistLabel}.plist`);

    if (fs.existsSync(plistPath)) {
      try { execSync(`launchctl unload -w "${plistPath}"`, { stdio: 'inherit' }); } catch (_) {}
      try { execSync(`pkill -f figma-rust-mcp 2>/dev/null`, { stdio: 'pipe' }); } catch (_) {}
      fs.unlinkSync(plistPath);
    }
    removeLegacyService();

    ok('Background service removed');

  } else if (platform === 'linux') {
    try { execSync(`systemctl --user disable --now figma-rust-mcp`, { stdio: 'inherit' }); } catch (_) {}
    const servicePath = path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-rust-mcp.service');
    if (fs.existsSync(servicePath)) fs.unlinkSync(servicePath);
    try { execSync(`systemctl --user daemon-reload`, { stdio: 'inherit' }); } catch (_) {}
    removeLegacyService();

    ok('Background service removed');

  } else if (platform === 'win32') {
    const taskName = 'FigmaRustMCPServer';
    try { execSync(`schtasks /Delete /F /TN "${taskName}"`, { stdio: 'inherit' }); } catch (_) {}
    removeLegacyService();

    ok('Background service removed');

  } else {
    console.error(`\x1b[31mUnsupported platform: ${platform}\x1b[0m`);
    process.exit(1);
  }
}

function removeLegacyService() {
  if (process.platform === 'darwin') {
    const legacyPlist = path.join(os.homedir(), 'Library', 'LaunchAgents', 'io.github.figma-mcp.server.plist');
    if (fs.existsSync(legacyPlist)) {
      try { execSync(`launchctl unload -w "${legacyPlist}"`, { stdio: 'pipe' }); } catch (_) {}
      try { execSync('pkill -x figma-mcp', { stdio: 'pipe' }); } catch (_) {}
      fs.unlinkSync(legacyPlist);
    }
  } else if (process.platform === 'linux') {
    const legacyUnit = path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-mcp.service');
    if (fs.existsSync(legacyUnit)) {
      try { execSync('systemctl --user disable --now figma-mcp', { stdio: 'pipe' }); } catch (_) {}
      fs.unlinkSync(legacyUnit);
      try { execSync('systemctl --user daemon-reload', { stdio: 'pipe' }); } catch (_) {}
    }
  } else if (process.platform === 'win32') {
    try { execSync('schtasks /End /TN "FigmaMCPServer"', { stdio: 'pipe' }); } catch (_) {}
    try { execSync('schtasks /Delete /F /TN "FigmaMCPServer"', { stdio: 'pipe' }); } catch (_) {}
  }
}

/**
 * Check if the service is currently installed.
 */
function isServiceInstalled() {
  const platform = process.platform;
  if (platform === 'darwin') {
    const plistPath = path.join(os.homedir(), 'Library', 'LaunchAgents', 'io.github.figma-rust-mcp.server.plist');
    return fs.existsSync(plistPath) || fs.existsSync(path.join(os.homedir(), 'Library', 'LaunchAgents', 'io.github.figma-mcp.server.plist'));
  } else if (platform === 'linux') {
    const servicePath = path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-rust-mcp.service');
    return fs.existsSync(servicePath) || fs.existsSync(path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-mcp.service'));
  } else if (platform === 'win32') {
    try {
      execSync('schtasks /Query /TN "FigmaRustMCPServer"', { stdio: 'pipe' });
      return true;
    } catch (_) {
      try { execSync('schtasks /Query /TN "FigmaMCPServer"', { stdio: 'pipe' }); return true; }
      catch (_) { return false; }
    }
  }
  return false;
}

function setupPlugin(customDir) {
  const pluginDir = customDir || path.join(os.homedir(), '.figma-rust-mcp', 'plugin');
  const sourcePluginDir = path.resolve(__dirname, '../plugin');

  fs.mkdirSync(pluginDir, { recursive: true });

  const files = ['manifest.json', 'code.js', 'ui.html', 'icon16.png', 'icon32.png'];
  for (const f of files) {
    const src = path.join(sourcePluginDir, f);
    const dest = path.join(pluginDir, f);
    if (fs.existsSync(src)) {
      fs.copyFileSync(src, dest);
    }
  }

  const manifestPath = path.join(pluginDir, 'manifest.json');
  ok('Figma plugin installed', tilde(manifestPath));
  note('First time only: Figma Desktop → Plugins → Development → Import plugin from manifest…');
  note('Later updates load automatically; no re-import needed.');
  return manifestPath;
}

function isNewer(latest, current) {
  const pLatest = latest.split('.').map(n => parseInt(n, 10) || 0);
  const pCurrent = current.split('.').map(n => parseInt(n, 10) || 0);
  for (let i = 0; i < 3; i++) {
    if ((pLatest[i] || 0) > (pCurrent[i] || 0)) return true;
    if ((pLatest[i] || 0) < (pCurrent[i] || 0)) return false;
  }
  return false;
}

// --upgrade stays on the installed major; a new major is announced but never auto-installed.
function updateNotice(latest) {
  if (latest.split('.')[0] !== pkgVersion.split('.')[0]) {
    return `\n\x1b[36m⚡ v${latest} is a new major release with breaking changes (see CHANGELOG).\x1b[0m\n\x1b[36m   Opt in: npm install -g figma-rust-mcp@latest\x1b[0m\n\n`;
  }
  return `\n\x1b[36m⚡ Update available: v${pkgVersion} → v${latest}\x1b[0m\n\x1b[36m   Run: npx figma-rust-mcp --upgrade\x1b[0m\n\n`;
}

function checkForUpdatesAsync() {
  const updateStateFile = path.join(os.homedir(), '.figma-rust-mcp', 'update-check.json');
  try {
    if (fs.existsSync(updateStateFile)) {
      const state = JSON.parse(fs.readFileSync(updateStateFile, 'utf8'));
      const now = Date.now();
      if (state.lastChecked && (now - state.lastChecked < 6 * 3600 * 1000)) {
        if (state.latestVersion && isNewer(state.latestVersion, pkgVersion)) {
          process.stderr.write(updateNotice(state.latestVersion));
        }
        return;
      }
    }
  } catch (_) {}

  const req = https.get('https://registry.npmjs.org/figma-rust-mcp/latest', {
    headers: { 'User-Agent': 'figma-rust-mcp-updater' },
    timeout: 2000
  }, (res) => {
    let data = '';
    res.on('data', chunk => { data += chunk; });
    res.on('end', () => {
      try {
        if (res.statusCode === 200) {
          const json = JSON.parse(data);
          const latest = json.version;
          fs.mkdirSync(path.join(os.homedir(), '.figma-rust-mcp'), { recursive: true });
          fs.writeFileSync(updateStateFile, JSON.stringify({
            lastChecked: Date.now(),
            latestVersion: latest
          }, null, 2));
          if (latest && isNewer(latest, pkgVersion)) {
            process.stderr.write(updateNotice(latest));
          }
        }
      } catch (_) {}
    });
  });
  req.on('error', () => {});
  req.on('timeout', () => req.destroy());
}

async function upgrade() {
  console.log('\x1b[36m[figma-rust-mcp]\x1b[0m Checking for updates and upgrading figma-rust-mcp...');
  try {
    const major = pkgVersion ? pkgVersion.split('.')[0] : 'latest';
    execSync(`npm install -g figma-rust-mcp@${major}`, { stdio: 'inherit' });
    ok(`Upgraded figma-rust-mcp (${major === 'latest' ? 'latest' : `v${major}.x`})`);
    setupPlugin();
    if (isServiceInstalled()) {
      console.log('\x1b[36mUpdating background service binary...\x1b[0m');
      const binPath = await findOrDownloadBinary();
      await installService(binPath);
    }
  } catch (err) {
    console.log('\x1b[33mTip: Run directly with latest version:\x1b[0m\n  npx -y figma-rust-mcp@latest\n');
  }
}

async function setupMenu() {
  if (!process.stdin.isTTY || !process.stdout.isTTY) {
    console.log(`Figma Rust MCP setup options:\n  npx -y figma-rust-mcp@latest --install-service  Install plugin and background service\n  npx -y figma-rust-mcp@latest --setup-plugin     Install/update plugin only\n  npx -y figma-rust-mcp@latest --service-status    Check background service`);
    return;
  }

  const rl = createInterface({ input: process.stdin, output: process.stdout });
  const agents = detectAgents();
  const items = [
    ['1', 'Quick setup', 'plugin + background service + agents', true],
    ['2', 'Figma plugin', 'install or update only'],
    ['3', 'Run server', 'in this terminal'],
    ['4', 'Agents', `configure detected agents (${agents.length} found)`],
    ['5', 'Service status', 'is the background service installed?'],
    ['6', 'Upgrade', 'update package and refresh setup'],
    ['7', 'Remove service', 'stop and uninstall the background service'],
    ['0', 'Exit', ''],
  ];
  try {
    console.log(`\n  ${bold('Figma Rust MCP')} ${dim(`v${pkgVersion}`)}\n`);
    for (const [key, label, hint, recommended] of items) {
      console.log(`  ${cyan(key)}  ${label.padEnd(16)}${dim(hint)}${recommended ? ` ${green('(recommended)')}` : ''}`);
    }
    const choice = (await rl.question(`\n${cyan('›')} Choose [1]: `)).trim() || '1';
    switch (choice) {
      case '1': {
        step(1, 3, 'Figma plugin');
        setupPlugin();
        step(2, 3, 'Background service');
        const binPath = await findOrDownloadBinary();
        await installService(binPath);
        step(3, 3, 'AI agents');
        await configureDetectedAgents(rl, agents);
        console.log(`\n${green('✓')} ${bold('Setup complete.')} Restart your agents, then run the plugin in Figma.\n`);
        break;
      }
      case '2': setupPlugin(); break;
      case '3': await runBinary(['--server']); break;
      case '4': await configureDetectedAgents(rl, agents); note('Restart the configured agents to connect.'); break;
      case '5': isServiceInstalled() ? ok('Background service is installed') : console.log(`  ${yellow('!')} Background service is not installed`); break;
      case '6': await upgrade(); break;
      case '7': await uninstallService(); break;
      case '0': break;
      default: console.log(`  ${yellow('!')} Unknown option. Run --init to try again.`);
    }
  } finally {
    rl.close();
  }
}

async function configureDetectedAgents(rl, agents) {
  if (!agents.length) {
    console.log(`  ${yellow('!')} No supported AI agents detected. Install one and rerun --init.`);
    return;
  }

  const width = Math.max(...agents.map(agent => agent.label.length));
  agents.forEach((agent, index) => console.log(`  ${cyan(index + 1)}  ${agent.label.padEnd(width + 2)}${dim(tilde(agent.configPath))}`));
  const answer = (await rl.question(`\n${cyan('›')} Configure which? ${dim('numbers like 1,3 · a = all · n = skip')} [a]: `)).trim().toLowerCase() || 'a';
  if (answer === 'n') {
    note('Skipped; MCP configuration was not changed.');
    return;
  }
  const selected = answer === 'a'
    ? agents
    : [...new Set(answer.split(',').map(value => Number(value.trim()) - 1))]
      .filter(index => Number.isInteger(index) && index >= 0 && index < agents.length)
      .map(index => agents[index]);
  if (!selected.length) {
    console.log(`  ${yellow('!')} No valid selection; MCP configuration was not changed.`);
    return;
  }

  for (const agent of selected) {
    try {
      ok(agent.label.padEnd(width + 2), configureAgent(agent) ? 'configured' : 'already configured');
    } catch (err) {
      console.log(`  ${red('✗')} ${agent.label.padEnd(width + 2)}${dim(err.message)}`);
    }
  }
}

function runBinary(args) {
  return findOrDownloadBinary().then(binPath => new Promise((resolve, reject) => {
    const child = spawn(binPath, args, { stdio: 'inherit', env: process.env });
    child.on('error', reject);
    child.on('exit', (code, signal) => signal ? process.kill(process.pid, signal) : resolve(code ?? 0));
    for (const sig of ['SIGINT', 'SIGTERM']) process.once(sig, () => child.kill(sig));
  }));
}

function setupAlias() {
  const platform = process.platform;
  if (platform === 'win32') {
    console.log('\x1b[33mOn Windows, install globally for direct command access:\x1b[0m\n  npm install -g figma-rust-mcp\n');
    return;
  }

  const shellRc = process.env.SHELL && process.env.SHELL.includes('zsh')
    ? path.join(os.homedir(), '.zshrc')
    : path.join(os.homedir(), '.bashrc');

  const aliasLine = "alias figma-rust-mcp='npx figma-rust-mcp'";
  try {
    let content = fs.existsSync(shellRc) ? fs.readFileSync(shellRc, 'utf8') : '';
    if (!content.includes('alias figma-rust-mcp=')) {
      fs.appendFileSync(shellRc, `\n# Figma Rust MCP alias\n${aliasLine}\n`);
      console.log(`\x1b[32m✓ Added alias to ${shellRc}:\x1b[0m`);
      console.log(`  ${aliasLine}`);
      console.log(`\nRun \x1b[36msource ${shellRc}\x1b[0m to start using \x1b[1mfigma-rust-mcp\x1b[0m command directly!`);
    } else {
      console.log(`\x1b[32m✓ Alias already exists in ${shellRc}\x1b[0m`);
    }
  } catch (err) {
    console.error(`Failed to configure alias: ${err.message}`);
  }
}

// ─── MAIN ─────────────────────────────────────────────────────────────────────

async function main() {
  const args = process.argv.slice(2);

  if (args.includes('--init') || args.includes('--configure')) {
    try { await setupMenu(); }
    catch (err) { console.error(`\x1b[31m[figma-rust-mcp error]\x1b[0m ${err.message}`); process.exit(1); }
    return;
  }

  if (args.includes('--upgrade') || args.includes('--update')) {
    await upgrade();
    return;
  }

  if (args.includes('--alias')) {
    setupAlias();
    return;
  }

  if (args.includes('--setup-plugin') || args.includes('--export-plugin') || args.includes('--setup')) {
    const customDir = args.find(a => !a.startsWith('-') && a !== '--setup-plugin' && a !== '--export-plugin' && a !== '--setup');
    setupPlugin(customDir);
    return;
  }

  if (args.includes('--install-service')) {
    try {
      setupPlugin();
      const binPath = await findOrDownloadBinary();
      await installService(binPath);
    } catch (err) {
      console.error(`\x1b[31m[figma-rust-mcp error]\x1b[0m ${err.message}`);
      process.exit(1);
    }
    return;
  }

  if (args.includes('--uninstall-service')) {
    try {
      await uninstallService();
    } catch (err) {
      console.error(`\x1b[31m[figma-rust-mcp error]\x1b[0m ${err.message}`);
      process.exit(1);
    }
    return;
  }

  if (args.includes('--service-status')) {
    const installed = isServiceInstalled();
    if (installed) {
      console.log('\x1b[32m✓ figma-rust-mcp service is installed.\x1b[0m');
      console.log('  To uninstall: npx figma-rust-mcp --uninstall-service');
    } else {
      console.log('\x1b[33m✗ figma-rust-mcp service is NOT installed.\x1b[0m');
      console.log('  To install:   npx figma-rust-mcp --install-service');
    }
    return;
  }

  // Non-blocking auto update check
  checkForUpdatesAsync();

  try {
    process.exit(await runBinary(args));
  } catch (err) {
    console.error(`\x1b[31m[figma-rust-mcp error]\x1b[0m ${err.message}`);
    process.exit(1);
  }
}

main();
