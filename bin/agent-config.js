import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { applyEdits, modify, parse } from 'jsonc-parser';

const SERVER_NAME = 'figma-rust-mcp';
const HTTP_URL = 'http://127.0.0.1:41730/mcp';

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
    { id: 'vscode', label: 'VS Code', configPath: path.join(platform === 'darwin' ? support : platform === 'win32' ? roaming : xdg, 'Code', 'User', 'mcp.json'), installed: existsAny([path.join(home, '.vscode'), platform === 'darwin' ? path.join(support, 'Code') : null, platform === 'linux' ? path.join(xdg, 'Code') : null, platform === 'win32' ? path.join(local, 'Programs', 'Microsoft VS Code') : null, platform === 'win32' ? path.join(roaming, 'Code') : null, macApp('Visual Studio Code')]) || hasCommand('code', env, platform) },
    { id: 'copilot', label: 'Copilot CLI', configPath: path.join(env.COPILOT_HOME || path.join(home, '.copilot'), 'mcp-config.json'), installed: existsAny([env.COPILOT_HOME, path.join(home, '.copilot')]) || hasCommand('copilot', env, platform) },
    { id: 'gemini', label: 'Gemini CLI', configPath: path.join(home, '.gemini', 'settings.json'), installed: hasCommand('gemini', env, platform) },
    { id: 'opencode', label: 'opencode', configPath: path.join(xdg, 'opencode', 'opencode.json'), installed: existsAny([path.join(xdg, 'opencode'), path.join(home, '.opencode')]) || hasCommand('opencode', env, platform) },
    { id: 'zed', label: 'Zed', configPath: platform === 'darwin' ? path.join(support, 'Zed', 'settings.json') : platform === 'win32' ? path.join(roaming, 'Zed', 'settings.json') : path.join(xdg, 'zed', 'settings.json'), installed: existsAny([path.join(xdg, 'zed'), platform === 'darwin' ? path.join(support, 'Zed') : null, platform === 'win32' ? path.join(roaming, 'Zed') : null, macApp('Zed')]) || hasCommand('zed', env, platform) },
  ];
  return agents.filter(agent => agent.installed).map(({ installed, ...agent }) => agent);
}

/** Write via a temp file + rename so a crash never leaves the agent's config truncated. */
function writeAtomic(filePath, text) {
  const target = fs.existsSync(filePath) ? fs.realpathSync(filePath) : filePath; // keep dotfile symlinks
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const mode = fs.existsSync(target) ? fs.statSync(target).mode & 0o777 : 0o644;
  const temp = `${target}.${process.pid}.tmp`;
  fs.writeFileSync(temp, text, { mode });
  fs.renameSync(temp, target);
}

function upsertJsonc(filePath, rootKey, value) {
  const source = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '{}\n';
  const errors = [];
  const config = parse(source, errors, { allowTrailingComma: true, disallowComments: false });
  if (errors.length) throw new Error(`Invalid JSON/JSONC in ${filePath}; left it unchanged.`);
  const current = config?.[rootKey]?.[SERVER_NAME];
  if (current && Object.entries(value).every(([key, entryValue]) => JSON.stringify(current[key]) === JSON.stringify(entryValue))) return false;
  const merged = current && typeof current === 'object' && !Array.isArray(current) ? { ...current, ...value } : value;
  for (const key of ['command', 'args', 'serverUrl', 'httpUrl', 'url', 'type']) {
    if (!Object.hasOwn(value, key) && Object.hasOwn(merged, key)) delete merged[key];
  }
  const edits = modify(source, [rootKey, SERVER_NAME], merged, { formattingOptions: { insertSpaces: true, tabSize: 2, eol: '\n' } });
  if (edits.length) writeAtomic(filePath, applyEdits(source, edits));
  return edits.length > 0;
}

function upsertCodex(filePath) {
  const source = fs.existsSync(filePath) ? fs.readFileSync(filePath, 'utf8') : '';
  const lines = source.match(/[^\n]*\n|[^\n]+$/g) || [];
  const headers = ['[mcp_servers.figma-rust-mcp]', '[mcp_servers."figma-rust-mcp"]'];
  const start = lines.findIndex(line => headers.includes(line.replace(/#.*/, '').trim()));
  const urlLine = `url = ${JSON.stringify(HTTP_URL)}\n`;
  let output;
  if (start < 0) {
    output = `${source.trimEnd()}${source.trim() ? '\n\n' : ''}[mcp_servers.figma-rust-mcp]\n${urlLine}`;
  } else {
    let end = start + 1;
    while (end < lines.length && !/^\s*\[/.test(lines[end])) end++;
    // Replace only the transport keys; the user's other keys (enabled, timeouts) and comments stay.
    const kept = [];
    for (let i = start + 1; i < end; i++) {
      const transport = lines[i].match(/^\s*(url|command|args)\s*=\s*(.*)\n?$/);
      if (!transport) { kept.push(lines[i]); continue; }
      // ponytail: a multi-line array is skipped up to its closing ']' line; nested arrays are not expected here.
      if (transport[2].trim().startsWith('[') && !transport[2].includes(']')) while (i + 1 < end && !lines[++i].includes(']'));
    }
    const header = lines[start].endsWith('\n') ? lines[start] : `${lines[start]}\n`;
    lines.splice(start, end - start, header, urlLine, ...kept);
    output = lines.join('');
  }
  if (output === source) return false;
  writeAtomic(filePath, output);
  return true;
}

const ENTRIES = {
  antigravity: ['mcpServers', { serverUrl: 'http://127.0.0.1:41730/sse' }],
  claude: ['mcpServers', { type: 'http', url: HTTP_URL }],
  windsurf: ['mcpServers', { serverUrl: HTTP_URL }],
  vscode: ['servers', { type: 'http', url: HTTP_URL }],
  copilot: ['mcpServers', { type: 'http', url: HTTP_URL, tools: ['*'] }],
  gemini: ['mcpServers', { httpUrl: HTTP_URL }],
  opencode: ['mcp', { type: 'remote', url: HTTP_URL, enabled: true }],
  zed: ['context_servers', { url: HTTP_URL }],
};

export function configureAgent(agent) {
  if (agent.id === 'codex') return upsertCodex(agent.configPath);
  const [rootKey, entry] = ENTRIES[agent.id] || ['mcpServers', { url: HTTP_URL }];
  return upsertJsonc(agent.configPath, rootKey, entry);
}

export const MCP_SERVER_NAME = SERVER_NAME;
