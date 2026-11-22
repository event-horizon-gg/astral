import React from 'react';
import { Cloud, Monitor } from 'lucide-react';
import SetupComparison from '../../components/SetupComparison.jsx';
import { localTime } from '../../application/format.js';
import { useProfiles } from './SyncProfiles.jsx';
export default function ConflictReview({ dialog, busy, call, onClose }) {
  const profiles = useProfiles();
  const comparison = dialog.comparison;
  const profile = profiles?.options.find((p) => p.id === comparison?.profile)?.name;
  const choose = async (choice) => {
    const args = [choice, dialog.scope, dialog.revision];
    if (dialog.localHash || dialog.context) args.push(dialog.localHash);
    if (dialog.context) args.push(dialog.context);
    const result = await call('sync', ...args);
    if (result && !result.conflict && !result.incomplete) onClose();
  };
  return (
    <>
      <h2>Which setup should we keep?</h2>
      <p className="comparison-context">
        {comparison?.client === 'forever' ? 'Forever' : 'Retail'}
        {profile ? ' · ' + profile : ''}
      </p>
      <div className="conflict-sources">
        <section>
          <Monitor size={22} />
          <h3>This computer</h3>
          <p>{comparison?.local.device || 'Current saved settings'}</p>
          <strong>{comparison?.local.files ?? dialog.localFiles} settings items</strong>
        </section>
        <section>
          <Cloud size={22} />
          <h3>Cloud setup</h3>
          <p>{comparison?.remote.device || 'Computer not recorded'}</p>
          <strong>
            {comparison?.remote.updated || dialog.remoteTime
              ? localTime(comparison?.remote.updated || dialog.remoteTime)
              : 'Saved time unavailable'}
          </strong>
          <span>Version {dialog.revision}</span>
        </section>
      </div>
      {comparison ? (
        <SetupComparison
          rows={comparison.rows}
          localLabel="Current"
          remoteLabel="After cloud apply"
        />
      ) : (
        <p>Sync again to load a comparison of these setups.</p>
      )}
      <p className="change-consequence">
        The chosen setup becomes the cloud version. A local backup is saved before replacing
        settings.
      </p>
      <div className="modal-actions conflict-actions">
        <button className="text-button" disabled={busy} onClick={onClose}>
          Decide later
        </button>
        <button className="button secondary" disabled={busy} onClick={() => choose('cloud')}>
          Use cloud setup
        </button>
        <button className="button primary" disabled={busy} onClick={() => choose('local')}>
          Use this computer
        </button>
      </div>
    </>
  );
}
