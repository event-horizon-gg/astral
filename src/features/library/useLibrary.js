import { useEffect, useState } from 'react';
import { runUpdateQueue } from './update-queue.js';
import { errorMessage } from '../../application/errors.js';

export default function useLibrary({ s, client, call, notify, setSelected }) {
  const [queueBusy, setQueueBusy] = useState(false),
    [queueResult, setQueueResult] = useState(null);
  useEffect(() => {
    setQueueResult(null);
  }, [s.root]);
  const check = async () => {
    const result = await call('check');
    if (!result) return;
    const errors = result.addons.filter((addon) => addon.release?.error).length;
    notify(
      errors
        ? `Checked available sources. ${errors} release check${errors === 1 ? ' needs' : 's need'} attention.`
        : `Release checks complete. ${result.addons.filter((addon) => addon.status === 'update').length} update(s) available.`,
    );
  };
  const update = async (addon) => {
    const result = await call('update', addon.id);
    if (!result) return;
    const installed = result.addons.find((value) => value.id === addon.id);
    notify(
      addon.name +
        (installed?.version === addon.version
          ? ' is already up to date.'
          : ' is installed and backed up.'),
    );
    setSelected(installed);
  };
  const importZip = async (addon) => {
    const result = await call('importZip', addon.id);
    if (!result) return;
    setSelected(result.addons.find((value) => value.id === addon.id) || null);
    notify(addon.name + ' updated from ZIP. Previous version saved in Activity.');
  };
  const updateAll = async () => {
    if (!client.available) return call('update');
    setQueueBusy(true);
    setQueueResult(null);
    try {
      const result = await runUpdateQueue(
        s.addons.filter((addon) => addon.status === 'update'),
        client.update,
      );
      setQueueResult(result);
      if (result.failed) await client.refresh();
    } catch (error) {
      notify(errorMessage(error), true);
    } finally {
      setQueueBusy(false);
    }
  };
  return { queueBusy, queueResult, setQueueResult, check, update, importZip, updateAll };
}
