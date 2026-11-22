import { useEffect, useRef } from 'react';
import { registerDialog } from './components/dialog-behavior.js';
export default function useDialog(open, onClose) {
  const ref = useRef(null),
    close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (open && ref.current) return registerDialog(window, ref.current, () => close.current());
  }, [open]);
  return ref;
}
