import Select from '../../components/Select.jsx';
import React, { useState } from 'react';
import Dialog from '../../components/Dialog.jsx';
import { defaultProfiles } from '../account/profile-cache.js';

export default function ShareSetup(props) {
  if (props.share.mode !== 'choose') return <SetupFileForm {...props} />;
  return (
    <Dialog
      className="modal share-setup"
      busy={props.busy}
      onClose={props.onClose}
      labelledBy="share-setup-title"
      feedbackCommands={['setupExportInfo', 'openSetupImport']}
    >
      <h2 id="share-setup-title">Share setup</h2>
      <div className="share-choices">
        <button
          className="share-choice"
          disabled={props.busy}
          onClick={() => props.onChoose('export', 'setupExportInfo')}
        >
          <strong>Save setup file</strong>
          <span>Share your addons, UI and keybindings.</span>
        </button>
        <button
          className="share-choice"
          disabled={props.busy}
          onClick={() => props.onChoose('import', 'openSetupImport')}
        >
          <strong>Open setup file</strong>
          <span>Review a shared setup before applying it.</span>
        </button>
      </div>
      <div className="modal-actions">
        <button className="button secondary" disabled={props.busy} onClick={props.onClose}>
          Close
        </button>
      </div>
    </Dialog>
  );
}

function SetupFileForm({ share, s, busy, call, notify, onClose }) {
  const importing = share.mode === 'import';
  const [name, setName] = useState(
    'My ' +
      (defaultProfiles.find((profile) => profile.id === s.syncProfile)?.reviewSetup
        ? defaultProfiles.find((profile) => profile.id === s.syncProfile).name
        : 'WoW') +
      ' setup',
  );
  const [character, setCharacter] = useState(
    importing
      ? share.info.characters.length === 1
        ? share.info.characters[0].value
        : ''
      : share.info.characters[0]?.value || '',
  );
  const [include, setInclude] = useState(
    importing
      ? { ...share.info.includes, controller: false }
      : { settings: true, bindings: true, controller: false },
  );
  const [plan, setPlan] = useState(null);
  const addons = importing
    ? share.info.addonList.map((addon) => addon.name)
    : share.info.addonNames;
  const blocked = busy || s.running || (importing && share.info.character && !character);
  const update = (key, value) => {
    setPlan(null);
    setInclude((old) => ({ ...old, [key]: value }));
  };
  return (
    <Dialog
      as="form"
      className="modal share-setup"
      busy={busy}
      onClose={onClose}
      labelledBy="share-setup-title"
      feedbackCommands={['exportSetup', 'reviewSetupImport', 'applySetupImport']}
      onSubmit={async (event) => {
        event.preventDefault();
        if (blocked) return;
        if (!importing) {
          const result = await call('exportSetup', { name, character, include });
          if (result) {
            onClose();
            notify('Setup exported.');
          }
        } else if (!plan) {
          const result = await call('reviewSetupImport', share.info.token, character, include);
          if (result) setPlan(result);
        } else if (!plan.missing.length) {
          const result = await call('applySetupImport', plan.token, plan.review);
          if (result) {
            onClose();
            notify('Setup imported. Open WoW to use it.');
          } else setPlan(null);
        }
      }}
    >
      <h2 id="share-setup-title">{importing ? `Import ${share.info.name}?` : 'Export setup'}</h2>
      <div className="share-context">
        <span>{s.flavor === 'forever' ? 'WoW Forever' : 'WoW Retail'}</span>
        <span>
          {defaultProfiles.find((profile) => profile.id === s.syncProfile)?.name ||
            'Current profile'}
        </span>
      </div>
      {!plan && (
        <>
          {!importing && (
            <label className="share-field">
              Name
              <input
                value={name}
                maxLength={80}
                required
                disabled={busy}
                onChange={(event) => setName(event.target.value)}
              />
            </label>
          )}
          {(!importing || share.info.character) && (
            <label className="share-field">
              {importing ? 'Apply character settings to' : 'Character'}
              <Select
                value={character}
                disabled={busy}
                required={importing && !!share.info.character}
                onChange={(event) => {
                  setCharacter(event.target.value);
                  setPlan(null);
                }}
              >
                <option value="">
                  {importing ? 'Choose a character' : 'Account-wide settings only'}
                </option>
                {share.info.characters.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </Select>
            </label>
          )}
          <fieldset disabled={busy} className="share-options">
            <legend>Include</legend>
            {[
              ['settings', 'UI and addon settings'],
              ['bindings', 'Keybindings'],
              ['controller', 'Controller settings'],
            ]
              .filter(([key]) => !importing || share.info.includes[key])
              .map(([key, label]) => (
                <label key={key}>
                  {label}
                  <input
                    type="checkbox"
                    checked={include[key]}
                    onChange={(event) => update(key, event.target.checked)}
                  />
                </label>
              ))}
          </fieldset>
          {!importing && include.settings && (
            <p className="share-data-note">
              Addon settings can include character names and history.
            </p>
          )}
        </>
      )}
      <details className="share-addon-list">
        <summary>
          {addons.length} addon{addons.length === 1 ? '' : 's'}
        </summary>
        <ul>
          {addons.map((addon, index) => (
            <li key={index}>{addon}</li>
          ))}
        </ul>
      </details>
      {plan && (
        <>
          <p>
            {plan.changedSettings.length} settings files will change. Your current settings will be
            backed up.
          </p>
          {include.settings && <p>Account-wide addon settings will also be replaced.</p>}
          {plan.packages.length > 0 && (
            <div className="share-install-list">
              <h3>Addons to install</h3>
              <ul>
                {plan.packages.map((addon) => (
                  <li key={addon.folder}>{addon.name}</li>
                ))}
              </ul>
            </div>
          )}
          {plan.missing.length > 0 && (
            <div className="inline-note warning" role="alert">
              <strong>Connect or install these addons first</strong>
              <ul>
                {plan.missing.map((addon) => (
                  <li key={addon.folder}>{addon.name}</li>
                ))}
              </ul>
            </div>
          )}
          {plan.changes.length > 0 && (
            <div className="control-list">
              {plan.changes.map((addon) => (
                <div className="control-row" key={addon.id}>
                  <strong>{addon.name}</strong>
                  <span>
                    {addon.before === null ? 'Mixed' : addon.before ? 'On' : 'Off'} →{' '}
                    {addon.enabled === null ? 'Mixed' : addon.enabled ? 'On' : 'Off'}
                  </span>
                </div>
              ))}
            </div>
          )}
          {plan.changedSettings.length > 0 && (
            <details>
              <summary>Changed settings</summary>
              <ul>
                {plan.changedSettings.map((file) => (
                  <li key={file}>{file}</li>
                ))}
              </ul>
            </details>
          )}
        </>
      )}
      {s.running && (
        <p role="status">Close WoW before {importing ? 'importing' : 'exporting'} settings.</p>
      )}
      <div className="modal-actions">
        <button
          type="button"
          className="button secondary"
          disabled={busy}
          onClick={() => (plan ? setPlan(null) : onClose())}
        >
          {plan ? 'Back' : 'Cancel'}
        </button>
        <button
          className="button primary"
          type="submit"
          disabled={blocked || (!importing && !name.trim()) || !!plan?.missing.length}
        >
          {busy
            ? 'Working…'
            : importing
              ? plan
                ? 'Import setup'
                : 'Review import'
              : 'Export file'}
        </button>
      </div>
    </Dialog>
  );
}
