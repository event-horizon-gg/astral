import React, { useEffect, useState } from 'react';
import ProfileReview from './ProfileReview.jsx';
import { useRequestScope } from '../../application/useResource.js';
import { defaultProfiles } from './profile-cache.js';
export default function AddonChoices({ s, busy, call, notify }) {
  const [edits, setEdits] = useState({}),
    [plan, setPlan] = useState(null);
  const rows = s.addonChoices?.rows || [];
  const signature = JSON.stringify(rows);
  const requests = useRequestScope([s.root, s.account, s.syncProfile, signature]);
  useEffect(() => {
    setEdits({});
    setPlan(null);
  }, [s.root, s.account, s.syncProfile, signature]);
  const count = Object.keys(edits).length;
  return (
    <section className="cloud-panel" aria-label="Addons in this profile">
      <div className="section-heading">
        <h3>Addons in this profile</h3>
        {count > 0 && (
          <button className="text-button" disabled={busy} onClick={() => setEdits({})}>
            Discard changes
          </button>
        )}
      </div>
      {!s.account ? (
        <p>Open WoW once, then close it and select your local account.</p>
      ) : s.addonChoices?.error ? (
        <p role="alert">{s.addonChoices.error}</p>
      ) : rows.length === 0 ? (
        <p>No installed addons found.</p>
      ) : (
        <>
          {s.running && <p>Close WoW before changing addon choices.</p>}
          <div className="profile-addon-list">
            {rows.map((addon) => {
              const managed = addon.managed;
              const value = Object.hasOwn(edits, addon.id) ? edits[addon.id] : addon.enabled;
              return (
                <label className="profile-addon-row" key={addon.id}>
                  <span>
                    {addon.name}
                    {managed ? (
                      <small>{addon.managedReason}</small>
                    ) : (
                      value === null && <small>Varies by character</small>
                    )}
                  </span>
                  <input
                    type="checkbox"
                    aria-label={addon.name}
                    aria-checked={value === null ? 'mixed' : !!value}
                    checked={value === true}
                    ref={(node) => {
                      if (node) node.indeterminate = value === null;
                    }}
                    disabled={busy || s.running || managed}
                    onChange={(event) => {
                      const next = { ...edits };
                      if (event.target.checked === addon.enabled) delete next[addon.id];
                      else next[addon.id] = event.target.checked;
                      setEdits(next);
                    }}
                  />
                </label>
              );
            })}
          </div>
          <div className="modal-actions">
            <button
              className="button secondary"
              disabled={busy || s.running || !count}
              onClick={async () => {
                const result = await requests.run(() =>
                  call('previewProfile', s.syncProfile || defaultProfiles[0].id, edits),
                );
                if (result) setPlan(result);
              }}
            >
              Review changes{count > 0 ? ` (${count})` : ''}
            </button>
          </div>
        </>
      )}
      <ProfileReview
        plan={plan}
        busy={busy}
        onClose={() => {
          requests.invalidate();
          setPlan(null);
        }}
        onApply={async () => {
          const result = await call('applyProfile', plan.token);
          setPlan(null);
          if (result) {
            setEdits({});
            notify('Addon choices saved for this profile.');
          }
        }}
      />
    </section>
  );
}
