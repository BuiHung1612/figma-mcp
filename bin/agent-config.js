import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { applyEdits, modify, parse } from 'jsonc-parser';

const SERVER_NAME = 'figma-rust-mcp';
const HTTP_URL = 'http://127.0.0.1:38451/mcp';

function existsAny(paths) {
  return paths.some(candidate => candidate && fs.existsSync(candidate));
}

function hasCommand(name, env = process.env, platform = process.platform) {
  const extensions = platform === 'win32' ? (env.PATHEXT || '.EXE;.CMD;.BAT').split(';') : [''];
  return (env.PATH || '').split(path.delimiter).some(dir =>
    extensions.some(ext => fs.existsSync(path.join(dir, `${name}${ext}`))));
}

export function detectAgents({ home = os.homedir(), env = process.env, platform = process.platform } = {}) {
  const roaming = env.APPDATA || path.join(home, 'AppData', 'Roaming');
  const local = env.LOCALAPPDATA || path.join(home, 'AppData', 'Local');
  const support = path.join(home, 'Library', 'Application Support');
  const xdg = env.XDG_CONFIG_HOME || path.join(home, '.config');
  const macApp = name => platform === 'darwin' ? `/Applications/${name}.app` : null;
  const agents = [
    { id: 'codex', label: 'Codex', configPath: path.join(env.CODEX_HOME || path.join(home, '.codex'), 'config.toml'), installed: existsAny([env.CODEX_HOME, path.join(home, '.codex')]) || hasCommand('codex', env, platform) },
    { id: 'claude', label: 'Claude Code', configPath: path.join(env.CLAUDE_CONFIG_DIR || home, '.claude.json'), installed: existsAny([path.join(home, '.claude'), path.join(home, '.claude.json')]) || hasCommand('claude', env, platform) },
    { id: 'antigravity', label: 'Google Antigravity', configPath: path.join(home, '.gemini', 'config', 'mcp_config.json'), installed: existsAny([path.join(home, '.antigravity-ide'), path.join(home, '.gemini', 'config', 'mcp_config.json'), platform === 'darwin' ? path.join(support, 'Antigravity') : null, platform === 'win32' ? path.join(local, 'Programs', 'Antigravity') : null, macApp('Antigravity')]) || hasCommand('antigravity', env, platform) },
    { id: 'cursor', label: 'Cursor', configPath: path.join(home, '.cursor', 'mcp.json'), installed: existsAny([path.join(home, '.cursor'), platform === 'darwin' ? path.join(support, 'Cursor') : null, platform === 'win32' ? path.join(local, 'Programs', 'Cursor') : null, macApp('Cursor')]) || hasCommand('cursor', env, platform) },
    { id: 'windsurf', label: 'Windsurf', configPath: path.join(home, '.codeium', 'windsurf', 'mcp_config.json'), installed: existsAny([path.join(home, '.codeium', 'windsurf'), platform === 'darwin' ? path.join(support, 'Windsurf') : null, platform === 'win32' ? path.join(local, 'Programs', 'Windsurf') : null, macApp('Windsurf')]) || hasCommand('windsurf', env, platform) },
    { id: 'vscode', label: 'VS Code / Copilot', configPath: path.join(home, '.copilot', 'mcp-config.json'), installed: existsAny([path.join(home, '.vscode'), path.join(home, '.copilot'), platform === 'darwin' ? path.join(support, 'Code') : null, platform === 'linux' ? path.join(xdg, 'Code') : null, platform === 'win32' ? path.join(local, 'Programs', 'Microsoft VS Code') : null, platform === 'win32' ? path.join(roaming, 'Code') : null, macApp('Visual Studio Code')]) || hasCommand('code', env, platform) },
    { id: 'zed', label: 'Zed', configPath: platform === 'darwin' ? path.join(support, 'Zed', 'settings.json') : platform === 'win32' ? path.join(roaming, 'Zed', 'settings.json') : path.join(xdg, 'zed', 'settings.json'), installed: existsAny([path.join(xdg, 'zed'), platform === 'darwin' ? path.join(support, 'Zed') : null, platform === 'win32' ? path.join(roaming, 'Zed') : null, macApp('Zed')]) || hasCommand('zed', env, platform) },
  ];
  return agents.filter(agent => agent.installed).map(({ installed, ...agent }) => agent);
}

function upsertJsonc(filePath, rootKey, value) {
  const source = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '{}\n';
  const errors = [];
  const config = parse(source, errors, { allowTrailingComma: true, disallowComments: false });
  if (errors.length) throw new Error(`Invalid JSON/JSONC in ${filePath}; left it unchanged.`);
  const current = config?.[rootKey]?.[SERVER_NAME];
  if (current && Object.entries(value).every(([key, entryValue]) => current[key] === entryValue)) return false;
  const merged = current && typeof current === 'object' && !Array.isArray(current) ? { ...current, ...value } : value;
  for (const key of ['command', 'args', 'serverUrl', 'url', 'type']) {
    if (!Object.hasOwn(value, key) && Object.hasOwn(merged, key)) delete merged[key];
  }
  const edits = modify(source, [rootKey, SERVER_NAME], merged, { formattingOptions: { insertSpaces: true, tabSize: 2, eol: '\n' } });
  if (edits.length) {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, applyEdits(source, edits));
  }
  return edits.length > 0;
}

function upsertCodex(filePath) {
  const source = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '';
  const lines = source.match(/[^\n]*\n|[^\n]+$/g) || [];
  const headers = ['[mcp_servers.figma-rust-mcp]', '[mcp_servers."figma-rust-mcp"]'];
  const start = lines.findIndex(line => headers.includes(line.trim()));
  const block = `[mcp_servers.figma-rust-mcp]\nurl = ${JSON.stringify(HTTP_URL)}\n`;
  let output;
  if (start < 0) {
    output = `${source.trimEnd()}${source.trim() ? '\n\n' : ''}${block}`;
  } else {
    let end = start + 1;
    while (end < lines.length && !/^\s*\[/.test(lines[end])) end++;
    const oldBlock = lines.slice(start, end).join('');
    const newBlock = `${block}\n`;
    if (oldBlock === newBlock || oldBlock === block) return false;
    lines.splice(start, end - start, newBlock);
    output = lines.join('');
  }
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, output);
  return true;
}

export function configureAgent(agent) {
  const entry = agent.id === 'antigravity'
    ? { serverUrl: 'http://127.0.0.1:38451/sse' }
    : agent.id === 'claude' || agent.id === 'vscode'
      ? { type: 'http', url: HTTP_URL }
      : { url: HTTP_URL };
  if (agent.id === 'codex') return upsertCodex(agent.configPath);
  const rootKey = agent.id === 'zed' ? 'context_servers' : 'mcpServers';
  return upsertJsonc(agent.configPath, rootKey, entry);
}

export const MCP_SERVER_NAME = SERVER_NAME;
