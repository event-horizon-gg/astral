import react from 'eslint-plugin-react';
export default [
  {
    files: ['src/**/*.{js,jsx}'],
    languageOptions: {
      ecmaVersion: 'latest',
      sourceType: 'module',
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: Object.fromEntries(
        [
          'window',
          'document',
          'localStorage',
          'fetch',
          'AbortController',
          'setTimeout',
          'clearTimeout',
        ].map((name) => [name, 'readonly']),
      ),
    },
    plugins: { react },
    rules: { 'no-undef': 'error', 'react/jsx-no-undef': 'error' },
  },
];
