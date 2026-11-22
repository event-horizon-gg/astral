import React, { useEffect, useRef, useState } from 'react';
import { Monitor, Gamepad2, Check, ChevronDown, Settings2 } from 'lucide-react';
import { useProfiles } from './SyncProfiles.jsx';

export default function ProfilePicker({ onManage, overlayOpen }) {
  const model = useProfiles();
  const [open, setOpen] = useState(false);
  const ref = useRef(null),
    trigger = useRef(null);
  useEffect(() => {
    const dismiss = (event) => {
      if (!ref.current?.contains(event.target)) setOpen(false);
    };
    const key = (event) => {
      if (event.key === 'Escape' && open) {
        setOpen(false);
        trigger.current?.focus();
      }
    };
    document.addEventListener('pointerdown', dismiss);
    document.addEventListener('keydown', key);
    return () => {
      document.removeEventListener('pointerdown', dismiss);
      document.removeEventListener('keydown', key);
    };
  }, [open]);
  useEffect(() => {
    if (overlayOpen || model?.overlayOpen) setOpen(false);
  }, [overlayOpen, model?.overlayOpen]);
  if (!model) return null;
  const { s, busy, options, active, preview, start } = model;
  const Icon = active === 'handheld' ? Gamepad2 : Monitor;
  const action = (callback) => {
    trigger.current?.focus();
    setOpen(false);
    callback();
  };
  return (
    <div className="header-popover-anchor setup-picker" ref={ref}>
      <button
        ref={trigger}
        className="setup-trigger"
        aria-label="Profile"
        aria-expanded={open}
        aria-controls="setup-profile-menu"
        disabled={!s.loaded}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon size={16} />
        <span>{options.find((p) => p.id === active)?.name || 'Selected profile'}</span>
        <ChevronDown size={13} />
      </button>
      {open && (
        <div
          className="header-popover setup-menu"
          id="setup-profile-menu"
          aria-label="Profiles"
        >
          <div className="setup-menu-title">
            {s.flavor === 'forever' ? 'Forever' : 'Retail'} profiles
          </div>
          {options.map((p) => {
            const Mark = p.id === 'handheld' ? Gamepad2 : Monitor;
            return (
              <button
                key={p.id}
                className={p.id === active ? 'selected-profile' : ''}
                aria-pressed={p.id === active}
                disabled={busy || s.running || !s.account}
                onClick={() =>
                  action(() => {
                    if (p.id !== active) preview(p.id);
                  })
                }
              >
                <Mark size={17} />
                <span>{p.name}</span>
                {p.id === active && <Check size={16} />}
              </button>
            );
          })}
          {(s.running || !s.account) && (
            <p className="setup-menu-hint">
              {s.running
                ? 'Close WoW to switch profiles.'
                : 'Select a local WoW account in Settings.'}
            </p>
          )}
          <button
            className="setup-manage"
            disabled={busy}
            onClick={() => action(() => (s.signedIn ? start('manage') : onManage()))}
          >
            <Settings2 size={16} />
            Manage profiles
          </button>
        </div>
      )}
    </div>
  );
}
