export function readTheme(storage) {
  try {
    return storage.getItem('astral-theme') === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}
export function saveTheme(storage, theme) {
  try {
    storage.setItem('astral-theme', theme);
    return true;
  } catch {
    return false;
  }
}
