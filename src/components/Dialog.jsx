import React from 'react';
import useDialog from '../useDialog.js';

export default function Dialog({
  open = true,
  busy = false,
  onClose,
  className = 'modal',
  label,
  labelledBy,
  as: Body = 'div',
  children,
  ...props
}) {
  const close = () => {
    if (!busy) onClose();
  };
  const ref = useDialog(open, close);
  if (!open) return null;
  return (
    <div className="modal-backdrop" onClick={close}>
      <Body
        {...props}
        ref={ref}
        className={className}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        aria-labelledby={labelledBy}
        onClick={(event) => event.stopPropagation()}
      >
        {children}
      </Body>
    </div>
  );
}
