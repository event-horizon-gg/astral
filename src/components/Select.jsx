import React, { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import { Check, ChevronDown } from 'lucide-react';

// The options remain ordinary React <option>s at call sites; the popup is app-owned.
export default function Select({
  children,
  value,
  onChange,
  disabled,
  className = '',
  required,
  ...props
}) {
  const options = React.Children.toArray(children)
    .filter(React.isValidElement)
    .map((child) => ({
      value: String(child.props.value ?? child.props.children),
      label: child.props.children,
      disabled: !!child.props.disabled,
    }));
  const selected = options.findIndex((item) => item.value === String(value));
  const [open, setOpen] = useState(false),
    [active, setActive] = useState(0),
    [position, setPosition] = useState({});
  const trigger = useRef(null),
    popup = useRef(null),
    search = useRef({ text: '', time: 0 });
  const id = useId();
  const enabled = options
    .map((item, index) => (!item.disabled ? index : -1))
    .filter((index) => index >= 0);
  const close = () => setOpen(false);
  const choose = (index) => {
    const item = options[index];
    if (!item || item.disabled) return;
    close();
    trigger.current?.focus();
    if (item.value !== String(value)) onChange?.({ target: { value: item.value } });
  };
  const show = () => {
    if (disabled || !enabled.length) return;
    setActive(selected >= 0 && !options[selected].disabled ? selected : enabled[0]);
    setOpen(true);
  };
  useEffect(() => {
    if (disabled) close();
  }, [disabled]);
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const rect = trigger.current.getBoundingClientRect();
      const width = Math.min(Math.max(rect.width, 180), window.innerWidth - 16);
      const below = window.innerHeight - rect.bottom - 12;
      const above = rect.top - 12;
      const down = below >= Math.min(280, options.length * 36 + 10) || below >= above;
      const height = Math.max(30, Math.min(280, down ? below : above));
      setPosition({
        left: Math.max(8, Math.min(rect.left, window.innerWidth - width - 8)),
        width,
        maxHeight: height,
        top: down ? rect.bottom + 6 : undefined,
        bottom: !down ? window.innerHeight - rect.top + 6 : undefined,
      });
    };
    place();
    popup.current.showPopover?.();
    const dismiss = (event) => {
      if (!trigger.current?.contains(event.target) && !popup.current?.contains(event.target))
        close();
    };
    const blur = (event) => {
      if (!trigger.current?.contains(event.target) && !popup.current?.contains(event.target))
        close();
    };
    document.addEventListener('pointerdown', dismiss);
    document.addEventListener('focusin', blur);
    window.addEventListener('resize', place);
    window.addEventListener('scroll', place, true);
    return () => {
      document.removeEventListener('pointerdown', dismiss);
      document.removeEventListener('focusin', blur);
      window.removeEventListener('resize', place);
      window.removeEventListener('scroll', place, true);
    };
  }, [open, options.length]);
  useEffect(() => {
    if (open)
      popup.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' });
  }, [open, active]);
  const keydown = (event) => {
    if (event.key === 'Tab') {
      close();
      return;
    }
    if (event.key === 'Escape' && open) {
      event.preventDefault();
      event.stopPropagation();
      close();
      return;
    }
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();
      if (!open) {
        show();
        return;
      }
      const current = enabled.indexOf(active);
      setActive(
        event.key === 'Home'
          ? enabled[0]
          : event.key === 'End'
            ? enabled.at(-1)
            : enabled[
                (current + (event.key === 'ArrowDown' ? 1 : -1) + enabled.length) % enabled.length
              ],
      );
    } else if (['Enter', ' '].includes(event.key)) {
      event.preventDefault();
      if (open) choose(active);
      else show();
    } else if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      const now = Date.now();
      search.current.text =
        now - search.current.time > 700 ? event.key : search.current.text + event.key;
      search.current.time = now;
      const text = search.current.text.toLowerCase();
      const match = enabled.find((index) =>
        String(options[index].label).toLowerCase().startsWith(text),
      );
      if (match !== undefined) {
        if (!open) show();
        setActive(match);
      }
    }
  };
  return (
    <>
      <button
        {...props}
        type="button"
        className={'app-select ' + className}
        ref={trigger}
        disabled={disabled}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-activedescendant={open ? id + '-' + active : undefined}
        aria-required={required || undefined}
        onKeyDown={keydown}
        onClick={() => (open ? close() : show())}
      >
        <span>{options[selected]?.label || 'Choose…'}</span>
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {open && (
        <div
          id={id}
          ref={popup}
          className="app-select-popup"
          popover="manual"
          role="listbox"
          aria-label={props['aria-label']}
          style={position}
          onPointerDown={(event) => event.preventDefault()}
        >
          {options.map((item, index) => (
            <div
              key={item.value}
              id={id + '-' + index}
              role="option"
              aria-selected={index === selected}
              aria-disabled={item.disabled || undefined}
              data-active={index === active}
              onPointerMove={() => !item.disabled && setActive(index)}
              onClick={() => choose(index)}
            >
              <span>{item.label}</span>
              {index === selected && <Check size={15} aria-hidden="true" />}
            </div>
          ))}
        </div>
      )}
    </>
  );
}
