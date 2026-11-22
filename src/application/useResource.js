import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createRequestScope } from './requests.js';
import { errorMessage } from './errors.js';

export function useRequestScope(key) {
  const identity = JSON.stringify(key);
  const scope = useMemo(() => createRequestScope(), [identity]);
  useEffect(() => () => scope.invalidate(), [scope]);
  return scope;
}

export default function useResource({ key, enabled = true, load, onValue, initial = null }) {
  const scope = useRequestScope(key);
  const latest = useRef({ load, onValue });
  latest.current = { load, onValue };
  const [state, setState] = useState({ value: initial, error: null, loading: false });
  const refresh = useCallback(async () => {
    if (!enabled) return;
    setState((old) => ({ ...old, error: null, loading: true }));
    const request = latest.current;
    try {
      return await scope.run(request.load, (value) => {
        setState({ value, error: null, loading: false });
        request.onValue?.(value);
      });
    } catch (error) {
      setState((old) => ({ ...old, error: errorMessage(error), loading: false }));
    }
  }, [enabled, scope]);
  useEffect(() => {
    setState({ value: initial, error: null, loading: false });
    if (enabled) refresh();
    return () => scope.invalidate();
  }, [enabled, scope, refresh]);
  return { ...state, refresh };
}
