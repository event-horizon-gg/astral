export const pages = [
  { id: 'Addons', label: 'Library', icon: 'Package', primary: true },
  { id: 'Discover', label: 'Discover', icon: 'Plus', primary: true },
  { id: 'Your setup', label: 'Profiles & sync', icon: 'Layers', account: true },
  {
    id: 'Activity',
    label: 'Activity',
    menuLabel: 'Activity & backups',
    icon: 'History',
    tools: true,
  },
  { id: 'Sources', label: 'Sources', icon: 'Globe', tools: true },
  { id: 'Settings', label: 'Settings', icon: 'Settings', tools: true },
];
export function pageLabel(id) {
  return pages.find((page) => page.id === id)?.label || id;
}
export function navigateState(state, page) {
  if (!pages.some((item) => item.id === page)) return state;
  return { ...state, page, selectedId: null, command: false };
}
export function selectedAddon(addons, selectedId) {
  return addons.find((addon) => addon.id === selectedId) || null;
}
