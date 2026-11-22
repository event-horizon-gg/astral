import React, { useContext } from 'react';
import useDialog from '../useDialog.js';
import FeedbackContext from './FeedbackContext.jsx';

export default function Dialog({
  open = true,
  busy = false,
  onClose,
  className = 'modal',
  backdropClassName = '',
  feedbackCommands = [],
  label,
  labelledBy,
  as: Body = 'div',
  children,
  ...props
}) {
  const feedback = useContext(FeedbackContext);
  const error =
    feedback?.toast?.error && feedbackCommands.includes(feedback.toast.command)
      ? feedback.toast.message
      : null;
  const close = () => {
    if (!busy) onClose();
  };
  const ref = useDialog(open, close);
  if (!open) return null;
  return (
    <div className={'modal-backdrop ' + backdropClassName} onClick={close}>
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
        {error && (
          <div role="alert" className="dialog-command-error">
            <span>{error}</span>
            <button type="button" className="text-button" onClick={feedback.onDismiss}>
              Dismiss
            </button>
          </div>
        )}
        {children}
      </Body>
    </div>
  );
}
