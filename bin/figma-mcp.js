#!/usr/bin/env node

/**
 * Figma MCP - NPX Binary Runner
 * Automatically detects platform/arch, downloads prebuilt Rust binary from GitHub Releases,
 * caches it locally, and executes it transparently.
 */

import { spawn, execSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import https from 'node:https';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

// Read package.json version
const pkgPath = path.resolve(__dirname, '../package.json');
let pkgVersion = '2.8.3';
let repoOwner = 'BuiHung1612';
let repoName = 'figma-mcp';

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

function getPlatformArch() {
  const platform = process.platform;
  const arch = process.arch;

  let osName = '';
  let archName = '';
  let ext = 'tar.gz';
  let binName = 'figma-mcp';

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
    binName = 'figma-mcp.exe';
  } else {
    throw new Error(`Unsupported operating system: ${platform}`);
  }

  const assetName = `figma-mcp-${osName}-${archName}.${ext}`;
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

  return path.join(baseCache, 'figma-mcp', `v${version}`);
}

function downloadFile(url, destPath) {
  return new Promise((resolve, reject) => {
    const followRedirects = (currentUrl, redirectCount = 0) => {
      if (redirectCount > 10) {
        reject(new Error('Too many redirects while downloading binary'));
        return;
      }

      const client = currentUrl.startsWith('https') ? https : http;
      client.get(currentUrl, { headers: { 'User-Agent': 'figma-mcp-npx' } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          followRedirects(res.headers.location, redirectCount + 1);
          return;
        }

        if (res.statusCode !== 200) {
          reject(new Error(`Failed to download binary: HTTP ${res.statusCode} from ${currentUrl}`));
          return;
        }

        const fileStream = fs.createWriteStream(destPath);
        res.pipe(fileStream);

        fileStream.on('finish', () => {
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
  if (process.env.FIGMA_MCP_BIN && fs.existsSync(process.env.FIGMA_MCP_BIN)) {
    return process.env.FIGMA_MCP_BIN;
  }

  const localRelease = path.resolve(__dirname, '../target/release/figma-mcp' + (process.platform === 'win32' ? '.exe' : ''));
  const localDebug = path.resolve(__dirname, '../target/debug/figma-mcp' + (process.platform === 'win32' ? '.exe' : ''));

  if (fs.existsSync(localRelease)) return localRelease;
  if (fs.existsSync(localDebug)) return localDebug;

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

  process.stderr.write(`\x1b[36m[figma-mcp]\x1b[0m Downloading binary for ${info.osName}-${info.archName} (v${pkgVersion})...\n`);

  try {
    await downloadFile(releaseUrl, archivePath);
  } catch (err) {
    process.stderr.write(`\x1b[33m[figma-mcp]\x1b[0m Release v${pkgVersion} not found, trying latest release...\n`);
    try {
      await downloadFile(latestUrl, archivePath);
    } catch (fallbackErr) {
      throw new Error(`Failed to download figma-mcp prebuilt binary:\n  - ${err.message}\n  - ${fallbackErr.message}\n\nPlease verify your internet connection or build from source: cargo build --release`);
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

  process.stderr.write(`\x1b[32m[figma-mcp]\x1b[0m Ready! (cached in ${cacheDir})\n\n`);
  return cachedBin;
}

// ─── SERVICE MANAGEMENT ───────────────────────────────────────────────────────

/**
 * Install figma-mcp as a background service that auto-starts on login.
 * - macOS: LaunchAgent plist (~/.config/figma-mcp/launchd.plist → ~/Library/LaunchAgents/)
 * - Linux: systemd user service (~/.config/systemd/user/figma-mcp.service)
 * - Windows: Task Scheduler XML via schtasks
 */
async function installService(binPath) {
  const platform = process.platform;
  const nodeExec = process.execPath;
  const scriptPath = path.resolve(__filename);

  if (platform === 'darwin') {
    const plistLabel = 'io.github.figma-mcp.server';
    const plistDir = path.join(os.homedir(), 'Library', 'LaunchAgents');
    const plistPath = path.join(plistDir, `${plistLabel}.plist`);

    fs.mkdirSync(plistDir, { recursive: true });

    const logDir = path.join(os.homedir(), 'Library', 'Logs', 'figma-mcp');
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

    // Unload first (ignore error if not loaded)
    try { execSync(`launchctl unload "${plistPath}" 2>/dev/null`, { stdio: 'pipe' }); } catch (_) {}
    execSync(`launchctl load -w "${plistPath}"`, { stdio: 'inherit' });

    console.log(`\x1b[32m✓ figma-mcp service installed!\x1b[0m`);
    console.log(`  Plist: ${plistPath}`);
    console.log(`  Logs:  ${logDir}/stdout.log`);
    console.log(`\n\x1b[36mThe server will now start automatically on every login.\x1b[0m`);
    console.log(`\x1b[36mTo uninstall: npx figma-mcp --uninstall-service\x1b[0m`);

  } else if (platform === 'linux') {
    const serviceDir = path.join(os.homedir(), '.config', 'systemd', 'user');
    const servicePath = path.join(serviceDir, 'figma-mcp.service');

    fs.mkdirSync(serviceDir, { recursive: true });

    const unit = `[Unit]
Description=Figma MCP Bridge Server
After=network.target

[Service]
Type=simple
ExecStart=${binPath}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
`;

    fs.writeFileSync(servicePath, unit, 'utf8');
    execSync(`systemctl --user daemon-reload`, { stdio: 'inherit' });
    execSync(`systemctl --user enable --now figma-mcp`, { stdio: 'inherit' });

    console.log(`\x1b[32m✓ figma-mcp systemd user service installed!\x1b[0m`);
    console.log(`  Service: ${servicePath}`);
    console.log(`\n\x1b[36mThe server will now start automatically on every login.\x1b[0m`);
    console.log(`\x1b[36mTo uninstall: npx figma-mcp --uninstall-service\x1b[0m`);

  } else if (platform === 'win32') {
    const taskName = 'FigmaMCPServer';
    // Use schtasks to create a task that runs at login
    const cmd = `schtasks /Create /F /TN "${taskName}" /TR "${binPath}" /SC ONLOGON /RL HIGHEST`;
    execSync(cmd, { stdio: 'inherit' });

    // Start it immediately too
    try { execSync(`schtasks /Run /TN "${taskName}"`, { stdio: 'pipe' }); } catch (_) {}

    console.log(`\x1b[32m✓ figma-rust-mcp Windows Task installed!\x1b[0m`);
    console.log(`  Task: ${taskName}`);
    console.log(`\n\x1b[36mThe server will now start automatically on every login.\x1b[0m`);
    console.log(`\x1b[36mTo uninstall: npx figma-rust-mcp --uninstall-service\x1b[0m`);

  } else {
    console.error(`\x1b[31mUnsupported platform: ${platform}\x1b[0m`);
    process.exit(1);
  }
}

/**
 * Uninstall the background service.
 */
async function uninstallService() {
  const platform = process.platform;

  if (platform === 'darwin') {
    const plistLabel = 'io.github.figma-mcp.server';
    const plistPath = path.join(os.homedir(), 'Library', 'LaunchAgents', `${plistLabel}.plist`);

    if (!fs.existsSync(plistPath)) {
      console.log(`\x1b[33mService not found at ${plistPath}\x1b[0m`);
      process.exit(0);
    }

    try { execSync(`launchctl unload -w "${plistPath}"`, { stdio: 'inherit' }); } catch (_) {}
    fs.unlinkSync(plistPath);

    console.log(`\x1b[32m✓ figma-rust-mcp service uninstalled.\x1b[0m`);

  } else if (platform === 'linux') {
    try { execSync(`systemctl --user disable --now figma-mcp`, { stdio: 'inherit' }); } catch (_) {}
    const servicePath = path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-mcp.service');
    if (fs.existsSync(servicePath)) fs.unlinkSync(servicePath);
    try { execSync(`systemctl --user daemon-reload`, { stdio: 'inherit' }); } catch (_) {}

    console.log(`\x1b[32m✓ figma-rust-mcp service uninstalled.\x1b[0m`);

  } else if (platform === 'win32') {
    const taskName = 'FigmaMCPServer';
    try { execSync(`schtasks /Delete /F /TN "${taskName}"`, { stdio: 'inherit' }); } catch (_) {}

    console.log(`\x1b[32m✓ figma-rust-mcp Windows Task removed.\x1b[0m`);

  } else {
    console.error(`\x1b[31mUnsupported platform: ${platform}\x1b[0m`);
    process.exit(1);
  }
}

/**
 * Check if the service is currently installed.
 */
function isServiceInstalled() {
  const platform = process.platform;
  if (platform === 'darwin') {
    const plistPath = path.join(os.homedir(), 'Library', 'LaunchAgents', 'io.github.figma-mcp.server.plist');
    return fs.existsSync(plistPath);
  } else if (platform === 'linux') {
    const servicePath = path.join(os.homedir(), '.config', 'systemd', 'user', 'figma-mcp.service');
    return fs.existsSync(servicePath);
  } else if (platform === 'win32') {
    try {
      execSync('schtasks /Query /TN "FigmaMCPServer"', { stdio: 'pipe' });
      return true;
    } catch (_) { return false; }
  }
  return false;
}

// ─── MAIN ─────────────────────────────────────────────────────────────────────

async function main() {
  const args = process.argv.slice(2);

  if (args.includes('--install-service')) {
    try {
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

  try {
    const binPath = await findOrDownloadBinary();

    const child = spawn(binPath, args, {
      stdio: 'inherit',
      env: process.env,
    });

    child.on('error', (err) => {
      console.error(`[figma-mcp] Failed to start binary: ${err.message}`);
      process.exit(1);
    });

    child.on('exit', (code, signal) => {
      if (signal) {
        process.kill(process.pid, signal);
      } else {
        process.exit(code ?? 0);
      }
    });

    const forwardSignal = (sig) => {
      if (child && !child.killed) {
        child.kill(sig);
      }
    };

    process.on('SIGINT', () => forwardSignal('SIGINT'));
    process.on('SIGTERM', () => forwardSignal('SIGTERM'));
  } catch (err) {
    console.error(`\x1b[31m[figma-mcp error]\x1b[0m ${err.message}`);
    process.exit(1);
  }
}

main();
