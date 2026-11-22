import React from 'react';
import { ArrowDownToLine, RefreshCw, Check, Settings, ArrowRight } from 'lucide-react';
import { pageLabel } from '../application/navigation.js';
export default function PageHeader({
  page,
  s,
  busy,
  updates,
  check,
  updateAll,
  navigate,
  api,
  progress,
}) {
  return (
    <>
      <div className="page-heading">
        <div>
          <h1>{pageLabel(page)}</h1>
          {page === 'Addons' && s.loaded && <p>{s.addons.length} addons</p>}
        </div>
        {page === 'Addons' && (
          <div className="page-actions">
            <button className="button secondary" disabled={busy} onClick={check}>
              <RefreshCw size={15} className={busy ? 'spin' : ''} />
              {progress.kind === 'check' ? 'Checking…' : 'Check for updates'}
            </button>
            {updates > 0 && (
              <button className="button primary" disabled={busy || s.running} onClick={updateAll}>
                <ArrowDownToLine size={15} />
                Update all ({updates})
              </button>
            )}
            {updates > 0 && s.running && (
              <span className="update-wait" role="status">
                Close WoW to update.
              </span>
            )}
          </div>
        )}
      </div>
      {page !== 'Settings' && ['available', 'ready'].includes(s.appUpdates?.phase) && (
        <div className="legacy-backups app-update-notice" role="status">
          <strong>Astral {s.appUpdates.latest} is available</strong>
          <button className="text-button" onClick={() => navigate('Settings')}>
            View update <ArrowRight size={14} />
          </button>
        </div>
      )}
      {!api && (
        <div className="preview-banner">
          Browser preview · real local inventory · actions are available in the desktop app
        </div>
      )}
    </>
  );
}
