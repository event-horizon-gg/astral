import React from 'react';
import { AlertCircle, CheckCircle2, X } from 'lucide-react';
export default function Notification({ toast, onDismiss }) {
  if (!toast) return null;
  return (
    <div
      className={'toast ' + (toast.error ? 'error' : '')}
      role={toast.error ? 'alert' : 'status'}
    >
      {toast.error ? <AlertCircle size={17} /> : <CheckCircle2 size={17} />}
      <span>{toast.message}</span>
      <button className="icon-button" aria-label="Dismiss notification" onClick={onDismiss}>
        <X size={14} />
      </button>
    </div>
  );
}
