import React from 'react';
import Dialog from '../../components/Dialog.jsx';
import SetupComparison from '../../components/SetupComparison.jsx';
export default function ProfileReview({ plan, name, busy, onClose, onApply, onRefresh }) {
  if (!plan) return null;
  const label = (value) => (value === null ? 'Varies by character' : value ? 'On' : 'Off');
  return (
    <Dialog
      className="modal profile-review"
      busy={busy}
      onClose={onClose}
      labelledBy="profile-review-title"
      feedbackCommands={['applyProfile', 'previewProfile']}
    >
      <h2 id="profile-review-title">{name ? `Switch to ${name}?` : 'Apply addon choices?'}</h2>
      {plan.initialized && name && <span className="mini-tag">New local setup</span>}
      <SetupComparison
        rows={(plan.comparison || []).filter((row) => row.changed && row.id !== 'choices')}
        showFiles={false}
        localLabel="Current"
        remoteLabel={name || 'After applying'}
      />
      {plan.packages?.length > 0 && (
        <section className="profile-packages">
          <h3>Addons to install</h3>
          <ul>
            {plan.packages.map((a) => (
              <li key={a.id || a.folder}>{a.name}</li>
            ))}
          </ul>
        </section>
      )}
      {plan.handheld && (
        <div className="control-row profile-bar-policy">
          <strong>Desktop action bars</strong>
          <span>Hidden</span>
        </div>
      )}
      {plan.handheld === 'forever' && (
        <div className="control-row">
          <strong>Native controller HUD</strong>
          <span>Kept</span>
        </div>
      )}
      {plan.missing?.length > 0 && (
        <p role="alert">
          Connect or install first: {plan.missing.map((a) => a.name).join(', ')}.
        </p>
      )}
      {plan.changes.length > 0 ? (
        <div className="control-list">
          {plan.changes.map((addon) => (
            <div className="control-row" key={addon.id}>
              <strong>{addon.name}</strong>
              <span>
                {label(addon.before)} → {label(addon.enabled)}
              </span>
            </div>
          ))}
        </div>
      ) : null}
      {plan.changedSettings.length > 0 && (
        <details>
          <summary>Changed settings · {plan.changedSettings.length}</summary>
          <ul>
            {plan.changedSettings.map((file) => (
              <li key={file}>{file}</li>
            ))}
          </ul>
        </details>
      )}
      <p className="change-consequence">Your current setup will be saved before applying.</p>
      <div className="modal-actions">
        <button className="button secondary" disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={busy || (!plan.stale && !!plan.missing?.length)}
          onClick={plan.stale ? onRefresh : onApply}
        >
          {plan.stale ? 'Review again' : name ? 'Save and switch' : 'Apply choices'}
        </button>
      </div>
    </Dialog>
  );
}
