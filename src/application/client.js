import commands from '../../shared/commands.json' with { type: 'json' };
import { errorMessage } from './errors.js';
import { createRequestScope } from './requests.js';

export function createApplicationClient(api, events = {}) {
  let pending = 0;
  const refreshes = createRequestScope();
  const publish = (state) => {
    refreshes.invalidate();
    events.state?.(state);
  };
  const read = async (name, ...args) => {
    if (!commands[name]) throw Error('Unknown Astral action.');
    if (!api)
      throw Error(
        'Open Astral’s desktop app to use this action. This browser preview is read-only.',
      );
    try {
      return await api[name](...args);
    } catch (error) {
      throw Error(errorMessage(error));
    }
  };
  const accept = (name, result, args = []) => {
    if (!result) return;
    const type = commands[name].result;
    if (type === 'state') publish(result);
    if (type === 'sync') {
      if (result.state) publish(result.state);
      if (result.conflict || result.incomplete) events.syncIssue?.(result, args);
      else publish(result);
    }
    if (result.syncMessage) events.message?.(result.syncMessage);
    events.result?.(name, result);
  };
  const refresh = async () => refreshes.run(() => read('state'), (value) => events.state?.(value));
  return {
    available: !!api,
    read,
    refresh,
    async execute(name, ...args) {
      events.before?.(name);
      pending++;
      events.busy?.(pending > 0);
      try {
        const result = await read(name, ...args);
        if (result?.canceled === true) return undefined;
        accept(name, result, args);
        return result;
      } catch (error) {
        events.error?.(errorMessage(error), name);
        if (api) {
          try {
            await refresh();
          } catch (refreshError) {
            events.refreshError?.(errorMessage(refreshError));
          }
        }
        return undefined;
      } finally {
        pending--;
        events.busy?.(pending > 0);
      }
    },
    async update(addon) {
      const state = await read('update', addon.id);
      accept('update', state);
      return state;
    },
    subscribe({ progress, changed }) {
      const stopProgress = api?.onProgress?.(progress);
      const stopChanged = api?.onChanged?.(changed);
      return () => {
        stopProgress?.();
        stopChanged?.();
        refreshes.invalidate();
      };
    },
  };
}
