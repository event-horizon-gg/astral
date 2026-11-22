import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createApplicationClient } from './client.js';
import { errorMessage } from './errors.js';

const empty = {
  addons: [],
  backups: [],
  accounts: [],
  providers: [],
  root: '',
  platform: 'darwin',
};

export default function useApplication(api) {
  const [s, setState] = useState(empty);
  const [requestBusy, setBusy] = useState(false);
  const [progress, setProgress] = useState({});
  const [toast, setToast] = useState(null);
  const [loadError, setLoadError] = useState(null);
  const [commandErrors, setCommandErrors] = useState({});
  const [dialog, setDialog] = useState(null);
  const toastTimer = useRef(null),
    automaticIssue = useRef(null);
  const notify = useCallback((message, error = false, command = null) => {
    setToast((current) => (current?.error && !error ? current : { message, error, command }));
    clearTimeout(toastTimer.current);
    if (!error)
      toastTimer.current = setTimeout(
        () => setToast((current) => (current?.error ? current : null)),
        6500,
      );
  }, []);
  useEffect(() => {
    setDialog(null);
    setToast(null);
    setCommandErrors({});
    automaticIssue.current = null;
  }, [s.root, s.account, s.flavor, s.syncProfile, s.accountName, s.signedIn]);
  useEffect(() => () => clearTimeout(toastTimer.current), []);
  const client = useMemo(
    () =>
      createApplicationClient(api, {
        state(value) {
          setState({ ...value, loaded: true });
          setLoadError(null);
          setToast((current) => (current?.command === 'state' ? null : current));
        },
        busy: setBusy,
        before(name) {
          setCommandErrors((errors) => ({ ...errors, [name]: null }));
        },
        error(message, name) {
          setCommandErrors((errors) => ({ ...errors, [name]: message }));
          notify(message, true, name);
        },
        refreshError: setLoadError,
        message: notify,
        syncIssue(result, args = []) {
          if (args[0] === 'auto') {
            const signature = JSON.stringify([result.scope, result.revision, result.missing]);
            if (automaticIssue.current === signature) return;
            automaticIssue.current = signature;
          }
          setDialog(result);
        },
        result(name, result) {
          if (!(name === 'sync' && (result.conflict || result.incomplete)))
            setToast((current) => (current?.command === name ? null : current));
          if (name === 'sync' && !result.conflict && !result.incomplete)
            automaticIssue.current = null;
        },
      }),
    [api, notify],
  );
  useEffect(() => {
    if (!client.available) {
      const abort = new AbortController();
      fetch('/preview.json', { signal: abort.signal })
        .then((response) => {
          if (!response.ok) throw Error('The preview could not be loaded.');
          return response.json();
        })
        .then((value) => setState({ ...value, loaded: true }))
        .catch((error) => {
          if (!abort.signal.aborted) setLoadError(errorMessage(error));
        });
      return () => abort.abort();
    }
    client.refresh().catch((error) => {
      setLoadError(errorMessage(error));
      notify(errorMessage(error), true, 'state');
    });
    return client.subscribe({
      progress(value) {
        setProgress(value);
        if (value.kind === 'sync-paused') notify(value.message, true, 'sync');
        if (value.kind === 'sync-ready') client.execute('sync', 'auto');
      },
      changed() {
        client.refresh().catch((error) => setLoadError(errorMessage(error)));
      },
    });
  }, [client, notify]);
  return {
    s,
    client,
    call: client.execute,
    requestBusy,
    progress,
    toast,
    dismissToast: () => setToast(null),
    notify,
    loadError,
    commandErrors,
    dialog,
    setDialog,
  };
}
