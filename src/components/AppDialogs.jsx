import React from 'react';
import { Search, ArrowRight, Package, Layers, History } from 'lucide-react';
import Dialog from './Dialog.jsx';
import { pages, pageLabel } from '../application/navigation.js';
import { navigationIcons } from './navigation-icons.js';
import { localTime as time } from '../application/format.js';
export default function AppDialogs({
  dialog,
  command,
  setDialog,
  setCommand,
  navigate,
  call,
  notify,
  busy,
}) {
  if (!dialog && !command) return null;
  const close = () => {
    setDialog(null);
    setCommand(false);
  };
  return (
    <Dialog
      open={!!dialog || command}
      busy={busy && !command}
      onClose={close}
      className={'modal ' + (command ? 'command-modal' : '')}
      label={command ? 'Quick find' : 'Review change'}
    >
      {command ? (
        <>
          <div className="command-title">
            <Search size={18} />
            <strong>Go to</strong>
            <kbd>ESC</kbd>
          </div>
          {pages
            .map((item) => [item.id, navigationIcons[item.icon]])
            .map(([p, Icon]) => (
              <button
                className="command-item"
                key={p}
                onClick={() => {
                  navigate(p);
                }}
              >
                <Icon size={18} />
                {pageLabel(p)}
                <ArrowRight size={15} />
              </button>
            ))}
        </>
      ) : dialog?.incomplete ? (
        <>
          <Package size={26} />
          <h2>Some addons need a source</h2>
          <p>Your current UI and bindings were kept. Install these addons, then sync again.</p>
          <div className="control-list">
            {dialog.missing.map((addon) => (
              <div className="control-row" key={addon.folder}>
                <strong>{addon.name}</strong>
                <span>{addon.reason}</span>
              </div>
            ))}
          </div>
          <div className="modal-actions">
            <button
              className="button primary"
              onClick={() => {
                setDialog(null);
                navigate('Your setup');
              }}
            >
              View shared addons
            </button>
          </div>
        </>
      ) : dialog?.conflict ? (
        <>
          <Layers size={26} />
          <h2>Choose which setup to keep</h2>
          <p>
            Choose what to sync with the selected profile. Astral saves a local backup before
            applying cloud settings.
          </p>
          <div className="modal-actions">
            <button
              className="button secondary"
              onClick={async () => {
                setDialog(null);
                await call('sync', 'cloud', dialog.scope, dialog.revision);
              }}
            >
              Use cloud setup
            </button>
            <button
              className="button primary"
              onClick={async () => {
                setDialog(null);
                await call('sync', 'local', dialog.scope, dialog.revision);
              }}
            >
              Use this computer
            </button>
          </div>
        </>
      ) : (
        <>
          <History size={26} />
          <h2>Restore {dialog.restore?.name}?</h2>
          <p>
            Restore the previous addon files from {time(dialog.restore?.time)}. Your saved settings
            stay in place.
          </p>
          <div className="modal-actions">
            <button className="button secondary" onClick={() => setDialog(null)}>
              Cancel
            </button>
            <button
              className="button primary"
              onClick={async () => {
                const id = dialog.restore.id;
                setDialog(null);
                const result = await call('restore', id);
                if (result) notify('Previous addon version restored.');
              }}
            >
              Restore previous
            </button>
          </div>
        </>
      )}
    </Dialog>
  );
}
