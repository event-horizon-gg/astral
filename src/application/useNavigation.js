import { useEffect, useState } from 'react';
import { navigateState, selectedAddon } from './navigation.js';

export default function useNavigation(s) {
  const [state, setState] = useState({ page: 'Addons', selectedId: null, command: false });
  const [query, setQuery] = useState(''),
    [filter, setFilter] = useState('All addons');
  const setSelected = (addon) => setState((old) => ({ ...old, selectedId: addon?.id || null }));
  const setCommand = (value) =>
    setState((old) => ({
      ...old,
      command: typeof value === 'function' ? value(old.command) : value,
    }));
  const navigate = (page) => setState((old) => navigateState(old, page));
  useEffect(() => {
    if (state.selectedId && !selectedAddon(s.addons, state.selectedId)) setSelected(null);
  }, [s.addons, state.selectedId]);
  return {
    ...state,
    selected: selectedAddon(s.addons, state.selectedId),
    setSelected,
    setCommand,
    navigate,
    query,
    setQuery,
    filter,
    setFilter,
  };
}
