const fs = require('node:fs/promises');
const crypto = require('node:crypto');
const engine = require('./engine.cjs');
const setupSharing = require('./setup-sharing.cjs');
const flavors = require('./flavors.cjs');
const profileContext = require('./profile-context.cjs');
const reviews = require('./reviews.cjs');
function register({ handle, runtime, state, persist, send, dialog, window, busy }) {
  const data = runtime.data;
  const imports = reviews.create({
    context: () => profileContext.selected(runtime.config),
    ttl: 300000,
    message: 'The selected setup changed. Open the import again.',
  });
  const setupContext = () => profileContext.selected(runtime.config);
  handle('setupExportInfo', async () => ({
    characters: await setupSharing.characters(runtime.config.root, runtime.config.account),
    addonNames: (await engine.inventory(runtime.config.root, data)).addons.map(({ name }) => name),
  }));
  handle('exportSetup', async (value) => {
    if (!value || typeof value !== 'object') throw Error('Choose a setup to export.');
    const context = setupContext();
    const bytes = await setupSharing.create({
      ...context,
      data,
      ...{
        name: value.name,
        character: value.character,
        include: value.include,
      },
    });
    const filename =
      value.name
        .trim()
        .replace(/[^\p{L}\p{N} _-]/gu, '')
        .slice(0, 70) || 'Astral setup';
    const result = await dialog.showSaveDialog(window(), {
      title: 'Export setup',
      buttonLabel: 'Export',
      defaultPath: filename + '.astral',
      filters: [{ name: 'Astral setup', extensions: ['astral'] }],
    });
    if (result.canceled || !result.filePath) return null;
    const temporary = result.filePath + '.' + crypto.randomUUID() + '.tmp';
    try {
      await fs.writeFile(temporary, bytes, { flag: 'wx', mode: 0o600 });
      await fs.rename(temporary, result.filePath);
    } finally {
      await fs.rm(temporary, { force: true });
    }
    return { exported: true };
  });
  handle('openSetupImport', async () => {
    const context = setupContext();
    const characters = await setupSharing.characters(runtime.config.root, runtime.config.account);
    const result = await dialog.showOpenDialog(window(), {
      title: 'Import setup',
      buttonLabel: 'Open',
      properties: ['openFile'],
      filters: [{ name: 'Astral setup', extensions: ['astral'] }],
    });
    if (result.canceled || !result.filePaths[0]) return null;
    const file = await fs.open(result.filePaths[0], 'r');
    let bytes;
    try {
      const stat = await file.stat();
      if (!stat.isFile() || stat.size > setupSharing.MAX_FILE)
        throw Error('Choose an Astral setup file smaller than 36 MB.');
      const buffer = Buffer.alloc(Math.min(stat.size + 1, setupSharing.MAX_FILE + 1));
      let length = 0;
      for (;;) {
        if (length === buffer.length)
          throw Error('The setup file changed while opening. Try again.');
        const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
        if (!bytesRead) break;
        length += bytesRead;
      }
      bytes = buffer.subarray(0, length);
    } finally {
      await file.close();
    }
    const share = setupSharing.decode(bytes);
    if (share.client !== flavors.flavor(runtime.config.root))
      throw Error(
        'Select WoW ' +
          (share.client === 'retail' ? 'Retail' : 'Forever') +
          ' before importing this setup.',
      );
    const token = await imports.replace({ ...context, share }, context);
    return {
      token,
      name: share.name,
      character: share.character,
      characters,
      addonList: share.bundle.library,
      includes: {
        settings: share.bundle.files.some((file) => !file.path.endsWith('bindings-cache.wtf')),
        bindings: share.bundle.files.some((file) => file.path.endsWith('bindings-cache.wtf')),
        controller: share.bundle.input !== undefined,
      },
    };
  });
  handle('reviewSetupImport', async (token, character, include) => {
    if (busy()) throw Error('Wait until the current operation finishes.');
    if (await engine.gameRunning()) throw Error('Close WoW before reviewing an import.');
    const value = await imports.get(token);
    const plan = await setupSharing.prepare({ ...value, data, character, include });
    if ((await imports.get(token)) !== value) throw Error('Open the import again.');
    value.character = character;
    value.include = setupSharing.options(include);
    value.expected = plan.fingerprint;
    value.review = crypto.randomUUID();
    return {
      token,
      review: value.review,
      name: value.share.name,
      packages: plan.packages.queue,
      missing: plan.packages.manual,
      changedSettings: plan.changedSettings,
      changes: plan.changes,
    };
  });
  handle('applySetupImport', (token, review) =>
    imports.run(token, async (value) => {
      if (!value.expected || value.review !== review)
        throw Error('Review the import before applying it.');
      const previous = structuredClone(runtime.config);
      try {
        await setupSharing.apply({
          ...value,
          data,
          onProgress: send,
          commit: async () => {
            runtime.config = profileContext.localChanges(runtime.config);
            try {
              await persist();
            } catch (error) {
              runtime.config = previous;
              throw error;
            }
          },
        });
        return await state();
      } finally {
        send({});
      }
    }),
  );
}
module.exports = { register };
