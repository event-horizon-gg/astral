import Select from '../../components/Select.jsx';
import React from 'react';
import { localTime as time } from '../../application/format.js';
import { Check, FolderOpen } from 'lucide-react';
export default function SettingsPage({ s, busy, call, notify }) {
  return (
    <div className="settings-list">
      <section>
        <h3>WoW: {s.flavor === 'forever' ? 'Forever' : 'Retail'}</h3>
        <div className="path-field">
          <FolderOpen size={17} />
          <code>{s.root || 'No game folder selected'}</code>
          <button className="text-button" onClick={() => call('chooseRoot')}>
            Change
          </button>
        </div>
        <label className="setting-label">
          Local WoW account
          <Select value={s.account || ''} onChange={(e) => call('setAccount', e.target.value)}>
            <option value="" disabled>
              Select account
            </option>
            {s.accounts.map((a) => (
              <option key={a}>{a}</option>
            ))}
          </Select>
        </label>
      </section>
      <section>
        <h3>Startup</h3>
        <label className="setting-label">
          Start Astral when I sign in
          <input
            type="checkbox"
            aria-label="Start Astral at login"
            checked={!!s.startup}
            onChange={(e) => call('startup', e.target.checked)}
          />
        </label>
      </section>
      <section>
        <h3>Addon updates</h3>
        <label className="setting-label">
          Update addons daily
          <input
            type="checkbox"
            aria-label="Daily addon updates"
            checked={!!s.autoUpdates}
            disabled={busy}
            onChange={(e) => call('autoUpdates', e.target.checked)}
          />
        </label>
        {(s.backgroundUpdates?.error || (s.autoUpdates && !s.backgroundUpdates?.enabled)) && (
          <p role="status">{s.backgroundUpdates?.error || 'Keep Astral open for daily updates.'}</p>
        )}
        <div className="update-actions">
          <button
            className="button secondary"
            disabled={busy || s.running}
            onClick={async () => {
              const r = await call('runDailyUpdates');
              if (r) notify('Daily update check finished. See Activity for results.');
            }}
          >
            Check now
          </button>
          <span className="muted">
            {s.running
              ? 'Waiting for WoW to close'
              : s.dailyUpdates?.runs?.length
                ? 'Last check ' + time(s.dailyUpdates.runs[0].time)
                : 'First daily check is pending'}
          </span>
        </div>
      </section>
      <section>
        <h3>Astral updates</h3>
        <p>Version {s.appVersion || '0.5.3'}</p>
        <label className="setting-label">
          Check daily
          <input
            type="checkbox"
            aria-label="Check for Astral updates automatically"
            checked={s.appUpdates?.enabled !== false}
            disabled={busy}
            onChange={(e) => call('appUpdateChecks', e.target.checked)}
          />
        </label>
        <p role="status" aria-live="polite">
          {s.appUpdates?.phase === 'checking'
            ? 'Checking for a new version…'
            : s.appUpdates?.phase === 'downloading'
              ? `Downloading… ${s.appUpdates.percent || 0}%`
              : s.appUpdates?.phase === 'available'
                ? `Astral ${s.appUpdates.latest} is available.`
                : s.appUpdates?.phase === 'ready'
                  ? `Astral ${s.appUpdates.latest} is ready to install.`
                  : s.appUpdates?.phase === 'current'
                    ? 'You’re up to date.'
                    : s.appUpdates?.phase === 'unpublished'
                      ? 'The first public release hasn’t been published yet.'
                      : s.appUpdates?.error || null}
        </p>
        <div className="update-actions">
          <button
            className="button secondary"
            disabled={busy || ['checking', 'downloading'].includes(s.appUpdates?.phase)}
            onClick={() => call('checkAppUpdate')}
          >
            Check for updates
          </button>
          {s.appUpdates?.phase === 'available' && (
            <button
              className="button primary"
              disabled={busy}
              onClick={() => call('downloadAppUpdate')}
            >
              Download update
            </button>
          )}
          {s.appUpdates?.phase === 'ready' && (
            <button
              className="button primary"
              disabled={busy || s.running}
              onClick={() => call('installAppUpdate')}
            >
              Update &amp; restart
            </button>
          )}
        </div>
        {s.appUpdates?.phase === 'ready' && s.running && (
          <p>Close WoW before installing the update.</p>
        )}
        {s.appUpdates?.notes && (
          <details className="release-notes-details">
            <summary>What’s new in {s.appUpdates.latest}</summary>
            <pre className="release-notes">{s.appUpdates.notes}</pre>
          </details>
        )}
      </section>
    </div>
  );
}
