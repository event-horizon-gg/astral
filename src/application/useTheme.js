import { useEffect, useState } from 'react';
import { readTheme, saveTheme } from './theme.js';
export default function useTheme() {
  const [theme, setTheme] = useState(() => readTheme(localStorage));
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    saveTheme(localStorage, theme);
  }, [theme]);
  return [theme, setTheme];
}
