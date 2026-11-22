const { contextBridge, ipcRenderer } = require('electron');
const argument = process.argv.findLast((value) => value.startsWith('--astral-commands='));
const actions = JSON.parse(
  decodeURIComponent(argument?.slice('--astral-commands='.length) || '[]'),
);
if (
  !Array.isArray(actions) ||
  !actions.length ||
  actions.some((name) => typeof name !== 'string' || !/^[a-z][A-Za-z]*$/.test(name))
)
  throw Error('Astral command contract is missing.');
const api = Object.fromEntries(
  actions.map((name) => [name, (...args) => ipcRenderer.invoke('astral:' + name, ...args)]),
);
api.onProgress = (fn) => {
  const listener = (_, value) => fn(value);
  ipcRenderer.on('astral:progress', listener);
  return () => ipcRenderer.removeListener('astral:progress', listener);
};
api.onChanged = (fn) => {
  const listener = () => fn();
  ipcRenderer.on('astral:changed', listener);
  return () => ipcRenderer.removeListener('astral:changed', listener);
};
contextBridge.exposeInMainWorld('astral', Object.freeze(api));
