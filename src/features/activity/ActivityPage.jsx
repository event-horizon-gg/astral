import React from 'react';
import { Check, History, FolderOpen, AlertCircle, ArrowUpRight } from 'lucide-react';
import { localTime as time } from '../../application/format.js';
export default function ActivityPage({ s, busy, call, notify, setDialog, navigate, update }) {
  return (
    <>
      {(s.addonActivity || []).length > 0 && (
        <section aria-label="Recent addon actions">
          <div className="section-heading">
            <h3>Recent actions</h3>
          </div>
          <div className="activity-list">
            {s.addonActivity.map((entry) => (
              <div className="activity-row" key={entry.id}>
                <span className="activity-icon">
                  {entry.status === 'error' ? <AlertCircle size={18} /> : <Check size={18} />}
                </span>
                <div className="activity-content">
                  <strong>
                    {entry.name} ·{' '}
                    {entry.status === 'error'
                      ? 'Failed'
                      : entry.status === 'current'
                        ? 'Already up to date'
                        : entry.action === 'restore'
                          ? 'Restored'
                          : 'Updated'}
                  </strong>
                  <p>
                    {entry.status === 'error'
                      ? entry.message
                      : `${entry.from || 'Not installed'} → ${entry.to || 'Installed'}`}
                  </p>
                  <p>{time(entry.time)}</p>
                </div>
                {entry.status === 'error' && entry.action === 'update' && (
                  <button
                    className="text-button"
                    disabled={busy || s.running}
                    onClick={() => {
                      const addon = s.addons.find((a) => a.id === entry.addonId);
                      if (addon) update(addon);
                    }}
                  >
                    Retry
                  </button>
                )}
              </div>
            ))}
          </div>
        </section>
      )}
      {s.legacyBackups && (
        <div className="legacy-backups">
          <div>
            <strong>Earlier Raider.IO backups</strong>
          </div>
          <button className="text-button" onClick={() => call('openLegacyBackups')}>
            Open archive
            <ArrowUpRight size={14} />
          </button>
        </div>
      )}
      {(s.dailyUpdates?.runs || []).slice(0, 3).map((run) => (
        <div className="update-run" key={run.time}>
          <h4>Daily update check · {time(run.time)}</h4>
          <p>
            {run.results.filter((r) => r.status === 'updated').length} updated ·{' '}
            {run.results.filter((r) => r.status === 'error' || r.status === 'review').length} need
            attention{run.status === 'deferred' ? ' · Waiting for WoW to close' : ''}
          </p>
          {run.results
            .filter((r) => r.status !== 'current')
            .map((r, i) => (
              <details key={r.id + '-' + i}>
                <summary>
                  {r.name} · {r.client === 'forever' ? 'Forever' : 'Retail'} ·{' '}
                  {r.message || (r.status === 'updated' ? `${r.from} → ${r.to}` : r.status)}
                </summary>
                {r.notes && <pre className="release-notes">{r.notes}</pre>}
              </details>
            ))}
        </div>
      ))}
      <div className="section-heading">
        <h3>Backups ({s.backups.length})</h3>
        <button className="button secondary" onClick={() => call('openBackups')}>
          <FolderOpen size={14} />
          Open backups
        </button>
      </div>
      {s.backups.length ? (
        <div className="activity-list">
          {s.backups.map((b) => (
            <div className="activity-row" key={b.id}>
              <span className="activity-icon">
                <History size={18} />
              </span>
              <div>
                <strong>{b.name} updated</strong>
                <p>
                  {b.version || 'First install'} → {b.release} · {time(b.time)}
                </p>
                {b.notes && (
                  <details className="release-notes-details">
                    <summary>Changelog</summary>
                    <pre className="release-notes">{b.notes}</pre>
                  </details>
                )}
              </div>
              <button
                className="text-button"
                disabled={busy || s.running}
                onClick={() => setDialog({ restore: b })}
              >
                Restore previous
              </button>
            </div>
          ))}
        </div>
      ) : (
        <div className="empty activity-empty">
          <History size={30} />
          <h3>No backups yet</h3>
          <button className="button secondary" onClick={() => navigate('Addons')}>
            Open library
          </button>
        </div>
      )}
    </>
  );
}
