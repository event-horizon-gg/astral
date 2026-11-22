import React, { useEffect, useState } from 'react';
import Dialog from '../../components/Dialog.jsx';
import useResource, { useRequestScope } from '../../application/useResource.js';
import {
  defaultProfiles as defaults,
  profileNameLimit,
  isCustomProfile,
  readProfileNames,
  saveProfileNames,
} from './profile-cache.js';
import ProfileReview from './ProfileReview.jsx';

export default function SyncProfiles({ s, busy, call, client }) {
  const [editing, setEditing] = useState(null),
    [name, setName] = useState('');
  const [refresh, setRefresh] = useState(0),
    [plan, setPlan] = useState(null);
  const active = s.syncProfile || defaults[0].id;
  const requests = useRequestScope([s.root, s.account, s.flavor, s.syncProfile]);
  const { value: profiles, error } = useResource({
    key: [s.signedIn, s.accountName, s.root, s.account, s.flavor, refresh],
    initial: readProfileNames(localStorage, s.root, s.account),
    enabled: s.signedIn && client.available,
    load: async () => (await client.read('syncProfiles')).profiles,
    onValue: (profiles) => saveProfileNames(localStorage, s.root, s.account, profiles),
  });
  const options = profiles || defaults;
  const start = (type, profile) => {
    if (type === 'manage' && !editing) setRefresh((value) => value + 1);
    setName(type === 'rename' ? profile.name : '');
    setEditing({ type, profile });
  };
  const save = async (event) => {
    event.preventDefault();
    const { type, profile } = editing;
    const result =
      type === 'delete'
        ? await call('deleteSyncProfile', profile.id)
        : type === 'rename'
          ? await call('renameSyncProfile', profile.id, name)
          : await call('createSyncProfile', name);
    if (!result) return;
    setRefresh((value) => value + 1);
    setEditing({ type: 'manage' });
  };

  return (
    <>
      <label className="sync-profile-picker">
        Profile
        <select
          value={active}
          disabled={busy || s.running || !s.account}
          onChange={async (event) => {
            const target = event.target.value;
            const result = await requests.run(() => call('previewProfile', target));
            if (result)
              setPlan({
                ...result,
                name: options.find((profile) => profile.id === target)?.name || 'profile',
              });
          }}
        >
          {!options.some((profile) => profile.id === active) && (
            <option value={active} disabled>
              {profiles ? 'Profile unavailable' : 'Selected profile'}
            </option>
          )}
          {options.map((profile) => (
            <option key={profile.id} value={profile.id}>
              {profile.name}
            </option>
          ))}
        </select>
      </label>
      {defaults.find((profile) => profile.id === active)?.reviewSetup && s.account && (
        <button
          className="text-button"
          disabled={busy || s.running}
          onClick={async () => {
            const result = await requests.run(() => call('previewProfile', active));
            if (result)
              setPlan({
                ...result,
                name:
                  options.find((profile) => profile.id === active)?.name ||
                  defaults.find((profile) => profile.id === active)?.name,
              });
          }}
        >
          Review Handheld setup
        </button>
      )}
      {!s.account && (
        <p className="control-hint">Open WoW once, then close it to set up profiles.</p>
      )}
      <ProfileReview
        plan={plan}
        name={plan?.name}
        busy={busy}
        onClose={() => {
          requests.invalidate();
          setPlan(null);
        }}
        onApply={async () => {
          await call('applyProfile', plan.token);
          setPlan(null);
        }}
      />
      {s.signedIn && (
        <button
          className="text-button profile-manager-button"
          disabled={busy}
          onClick={() => start('manage')}
        >
          Manage profiles
        </button>
      )}
      {editing && (
        <Dialog busy={busy} onClose={() => setEditing(null)} labelledBy="sync-profile-title">
          {editing.type === 'manage' ? (
            <>
              <h2 id="sync-profile-title">
                {s.flavor === 'forever' ? 'Forever' : 'Retail'} profiles
              </h2>
              {error && <p role="alert">{error}</p>}
              {!profiles && !error && <p>Loading profiles…</p>}
              <div className="control-list sync-profile-list">
                {profiles?.map((profile) => (
                  <div className="control-row" key={profile.id}>
                    <div>
                      <strong>{profile.name}</strong>
                      {profile.id === active && <span className="mini-tag">This computer</span>}
                    </div>
                    <div className="modal-actions">
                      <button
                        className="text-button"
                        disabled={busy}
                        onClick={() => start('rename', profile)}
                      >
                        Rename
                      </button>
                      {isCustomProfile(profile.id) && profile.id !== active && (
                        <button
                          className="text-button"
                          disabled={busy}
                          onClick={() => start('delete', profile)}
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
              <div className="modal-actions">
                <button
                  className="button secondary"
                  disabled={busy}
                  onClick={() => setEditing(null)}
                >
                  Done
                </button>
                {error ? (
                  <button
                    className="button primary"
                    onClick={() => setRefresh((value) => value + 1)}
                  >
                    Retry
                  </button>
                ) : (
                  <button
                    className="button primary"
                    disabled={busy || !profiles}
                    onClick={() => start('create')}
                  >
                    New profile
                  </button>
                )}
              </div>
            </>
          ) : (
            <form onSubmit={save}>
              <h2 id="sync-profile-title">
                {editing.type === 'delete'
                  ? 'Delete ' + editing.profile.name + '?'
                  : editing.type === 'rename'
                    ? 'Rename profile'
                    : 'New profile'}
              </h2>
              {editing.type === 'delete' ? (
                <p>
                  This permanently removes the profile and its cloud backups. Local WoW files stay
                  on your computers.
                </p>
              ) : (
                <label className="sync-profile-picker">
                  Name
                  <input
                    maxLength={profileNameLimit}
                    required
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                  />
                </label>
              )}
              <div className="modal-actions">
                <button
                  type="button"
                  className="button secondary"
                  disabled={busy}
                  onClick={() => start('manage')}
                >
                  Cancel
                </button>
                <button
                  type="submit"
                  className="button primary"
                  disabled={busy || (editing.type !== 'delete' && !name.trim())}
                >
                  {editing.type === 'delete'
                    ? 'Delete profile'
                    : editing.type === 'rename'
                      ? 'Save name'
                      : 'Create profile'}
                </button>
              </div>
            </form>
          )}
        </Dialog>
      )}
    </>
  );
}
