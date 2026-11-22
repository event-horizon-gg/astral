const windows = new WeakMap();
const selector =
  'button:not(:disabled),input:not(:disabled),select:not(:disabled),a[href],[tabindex="0"]';

export function registerDialog(window, element, onClose) {
  let owner = windows.get(window);
  if (!owner) {
    owner = { dialogs: [] };
    owner.keydown = (event) => {
      const current = owner.dialogs.at(-1);
      if (!current) return;
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        current.onClose();
      }
      if (event.key !== 'Tab') return;
      const items = [...current.element.querySelectorAll(selector)].filter(
        (item) => item.offsetParent !== null,
      );
      if (!items.length) {
        event.preventDefault();
        return;
      }
      const index = items.indexOf(window.document.activeElement);
      if (event.shiftKey && index <= 0) {
        event.preventDefault();
        items.at(-1).focus();
      } else if (!event.shiftKey && (index === items.length - 1 || index < 0)) {
        event.preventDefault();
        items[0].focus();
      }
    };
    window.addEventListener('keydown', owner.keydown, true);
    windows.set(window, owner);
  }
  const dialog = { element, onClose, previous: window.document.activeElement };
  owner.dialogs.push(dialog);
  [...element.querySelectorAll(selector)].find((item) => item.offsetParent !== null)?.focus();
  return () => {
    const wasTop = owner.dialogs.at(-1) === dialog;
    owner.dialogs = owner.dialogs.filter((item) => item !== dialog);
    if (wasTop && dialog.previous?.isConnected) dialog.previous.focus?.();
    if (!owner.dialogs.length) {
      window.removeEventListener('keydown', owner.keydown, true);
      windows.delete(window);
    }
  };
}
