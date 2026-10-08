import { readFileSync } from 'node:fs';
import vm from 'node:vm';

export function indexSyncUi(socket = { readyState: 1, send() {} }) {
  const html = readFileSync('plugin-runtime/ui.html', 'utf8');
  const element = () => {
    const classes = new Set();
    return { disabled: false, title: 'Sync index', attributes: {},
      setAttribute(name, value) { this.attributes[name] = value; },
      classList: { add: name => classes.add(name), remove: name => classes.delete(name),
        contains: name => classes.has(name), toggle: (name, active) => active ? classes.add(name) : classes.delete(name) } };
  };
  const timers = new Map();
  let timerId = 0;
  const ui = vm.createContext({ reindexBtn: element(), indexProgressWrap: element(), indexProgressText: element(),
    ws: socket, WebSocket: { OPEN: 1 }, log() {},
    setTimeout(callback, delay) { timers.set(++timerId, { callback, delay }); return timerId; },
    clearTimeout(id) { timers.delete(id); } });
  vm.runInContext(html.slice(html.indexOf('    var activeIndexScan ='), html.indexOf('    function focusSelection()')), ui);
  ui.fireTimers = delay => { for (const [id, timer] of timers) if (timer.delay === delay) { timers.delete(id); timer.callback(); } };
  return ui;
}
