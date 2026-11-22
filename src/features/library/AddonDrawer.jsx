import React from 'react';
import { ArrowDownToLine, Check, History, FolderOpen, ExternalLink, X } from 'lucide-react';
import AddonIcon from '../../components/AddonIcon.jsx';
export default function AddonDrawer({
  selected,
  setSelected,
  s,
  busy,
  call,
  update,
  importZip,
  check,
  selectedBackup,
  selectedFailure,
  setDialog,
}) {
  return (
    <>
      <div className="drawer-backdrop" onClick={() => setSelected(null)} />
      <aside className="detail-drawer" aria-label="Addon details">
        <button
          className="icon-button drawer-close"
          aria-label="Close details"
          onClick={() => setSelected(null)}
        >
          <X size={20} />
        </button>
        <AddonIcon addon={selected} large />
        <h2>{selected.name}</h2>
        <p className="drawer-desc">{selected.description}</p>
        <div className="detail-kv">
          <span>Installed version</span>
          <strong>{selected.version}</strong>
          <span>Source</span>
          <strong>{selected.local || selected.bundled ? 'Astral' : selected.source}</strong>
          {selected.release?.tag && (
            <>
              <span>Latest release</span>
              <strong>{selected.release.tag}</strong>
            </>
          )}
        </div>
        {selected.release?.error && (
          <div className="inline-note warning">{selected.release.error}</div>
        )}
        {selectedFailure?.status === 'error' && (
          <div className="inline-note warning" role="alert">
            Last attempt failed: {selectedFailure.message}
          </div>
        )}
        {(selected.repo || selected.bundled) && (
          <label className="addon-auto-toggle">
            Include in daily updates
            <input
              type="checkbox"
              checked={!(s.updateIgnored || []).includes(selected.id)}
              disabled={busy}
              onChange={(e) => call('autoIgnore', selected.id, !e.target.checked)}
            />
          </label>
        )}
        {(selected.repo || selected.bundled) && (
          <button
            className="button primary full"
            disabled={busy || (s.running && selected.status === 'update')}
            onClick={() => (selected.status === 'update' ? update(selected) : check())}
          >
            {selected.status === 'update' ? 'Update addon' : 'Check release'}
            <ArrowDownToLine size={15} />
          </button>
        )}
        {selected.canImport && (
          <button
            className="button secondary full"
            disabled={busy || s.running}
            onClick={() => importZip(selected)}
          >
            Update from ZIP
            <FolderOpen size={15} />
          </button>
        )}
        {selectedBackup && (
          <button
            className="button secondary full"
            disabled={busy || s.running}
            onClick={() => setDialog({ restore: selectedBackup })}
          >
            Restore {selectedBackup.version || 'previous version'}
            <History size={15} />
          </button>
        )}
        {!selected.bundled && (
          <button className="button secondary full" onClick={() => call('openSource', selected.id)}>
            {selected.repo ? 'View on GitHub' : 'Download page'}
            <ExternalLink size={14} />
          </button>
        )}
        {selected.status === 'manual' && (
          <div className="drawer-note">
            {selected.canImport
              ? 'Download the addon ZIP, then choose Update from ZIP.'
              : 'Update this addon manually. Changes made outside Astral are not backed up.'}
          </div>
        )}
        {s.running && (selected.status === 'update' || selected.canImport) && (
          <p role="status" className="inline-note warning">
            Close WoW before updating.
          </p>
        )}
        {selected.release?.notes && (
          <>
            <h4>Release notes</h4>
            <pre className="release-notes">{selected.release.notes}</pre>
          </>
        )}
      </aside>
    </>
  );
}
