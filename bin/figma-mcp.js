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
let pkgVersion = '2.6.0';
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

async function main() {
  try {
    const binPath = await findOrDownloadBinary();
    const args = process.argv.slice(2);

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
