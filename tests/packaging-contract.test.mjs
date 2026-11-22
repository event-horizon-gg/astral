import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { getFileMatchers } from 'app-builder-lib/out/fileMatcher.js';
import configuration from '../electron-builder.config.cjs';
const root = fileURLToPath(new URL('../', import.meta.url));

async function runtimeFiles(entry) {
  const files = new Set();
  async function visit(file) {
    if (files.has(file)) return;
    files.add(file);
    if (path.extname(file) === '.json') return;
    const source = await fs.readFile(file, 'utf8');
    const require = createRequire(file);
    for (const [, relative] of source.matchAll(/\brequire\(['"](\.[^'"]+)['"]\)/g))
      await visit(require.resolve(relative));
  }
  await visit(entry);
  return files;
}

test('Electron packaging includes every local runtime dependency of the main process', async () => {
  const metadata = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
  const require = createRequire(path.join(root, 'package.json'));
  const files = await runtimeFiles(require.resolve('./' + metadata.main));
  const destination = path.join(root, 'release', 'app');
  const matchers = getFileMatchers(configuration, 'files', destination, {
    defaultSrc: root,
    globalOutDir: path.join(root, 'release'),
    macroExpander: (value) => value,
    customBuildOptions: {},
  });
  const included = (matchers || []).map((matcher) => ({ matcher, filter: matcher.createFilter() }));
  for (const file of files) {
    const stat = await fs.stat(file);
    assert.ok(
      included.some(({ filter }) => filter(file, stat)),
      `Missing packaged runtime dependency: ${path.relative(root, file)}`,
    );
  }
});
