import { useEffect, useState } from 'react';
import { useRequestScope } from '../../application/useResource.js';

export default function useSharing(s, call) {
  const [share, setShare] = useState(null);
  const scope = useRequestScope([s.root, s.account, s.syncProfile]);
  useEffect(() => {
    setShare(null);
  }, [scope]);
  return {
    share,
    close() {
      scope.invalidate();
      setShare(null);
    },
    async open(mode, action) {
      const info = await scope.run(() => call(action));
      if (info) setShare({ mode, info });
    },
  };
}
