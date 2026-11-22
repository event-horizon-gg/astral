import React from 'react';
import useResource from '../../application/useResource.js';

export default function SharedLibrary({ s, busy, call, client }) {
  const {
    value: library,
    error,
    refresh: reload,
  } = useResource({
    key: [s.signedIn, s.accountName, s.root, s.flavor, s.lastSync],
    enabled: s.signedIn && client.available,
    load: () => client.read('sharedLibrary'),
  });
  const installed = new Set(
    s.addons
      .flatMap((addon) => addon.modules || [addon.folder])
      .map((folder) => folder.toLowerCase()),
  );
  const missing =
    library?.addons?.filter((addon) => !installed.has(addon.folder.toLowerCase())) || [];
  const sources = new Map(s.catalog?.map((addon) => [addon.folder.toLowerCase(), addon]));
  return (
    <section className="cloud-panel" aria-label="Shared addons">
      <div className="section-heading">
        <h3>Shared addons</h3>
        <button className="text-button" disabled={busy || s.running} onClick={() => call('sync')}>
          Sync addons
        </button>
        <button className="text-button" disabled={busy} onClick={reload}>
          Refresh
        </button>
      </div>
      {error ? (
        <p role="alert">{error}</p>
      ) : !library ? (
        <p>Loading library…</p>
      ) : (
        <>
          <p>
            {library.addons
              ? library.addons.length + ' shared · ' + missing.length + ' missing on this computer'
              : 'Addons sync across profiles for this game client.'}
          </p>
          {s.running && missing.length > 0 && <p>Close WoW to install addons.</p>}
          {missing.length > 0 && (
            <div className="control-list sync-profile-list">
              {missing.map((addon) => {
                const source = sources.get(addon.folder.toLowerCase());
                return (
                  <div className="control-row" key={addon.folder}>
                    <strong>{addon.name}</strong>
                    {source?.repo || source?.bundled ? (
                      <button
                        className="text-button"
                        disabled={busy || s.running}
                        onClick={() => call('update', source.id)}
                      >
                        Install
                      </button>
                    ) : source?.sourceUrl || source?.curseId ? (
                      <>
                        <button
                          className="text-button"
                          disabled={busy}
                          onClick={() => call('openSource', source.id)}
                        >
                          Download page
                        </button>
                        <button
                          className="text-button"
                          disabled={busy || s.running}
                          onClick={() => call('importZip', source.id)}
                        >
                          Install from ZIP
                        </button>
                      </>
                    ) : (
                      <span>Manual install</span>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </>
      )}
    </section>
  );
}
