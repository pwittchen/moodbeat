import { h } from './dom';

export interface ConfirmOptions {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel?: string;
}

export interface ConfirmDialog {
  el: HTMLElement;
  /** Resolves `true` when confirmed; Cancel, Escape and a backdrop click resolve `false`. */
  ask(opts: ConfirmOptions): Promise<boolean>;
  isOpen(): boolean;
}

export function createConfirmDialog(): ConfirmDialog {
  const title = h('h2', { class: 'dialog-title', id: 'confirm-title' });
  const message = h('p', { class: 'confirm-message', id: 'confirm-message' });
  const cancelBtn = h('button', { class: 'dark-pill', type: 'button' });
  const confirmBtn = h('button', { class: 'light-pill', type: 'button' });
  const dialog = h(
    'div',
    {
      class: 'dialog dialog--confirm',
      role: 'alertdialog',
      'aria-modal': 'true',
      'aria-labelledby': 'confirm-title',
      'aria-describedby': 'confirm-message',
    },
    title,
    message,
    h('div', { class: 'dialog-actions' }, cancelBtn, confirmBtn),
  );
  const el = h('div', { class: 'overlay', hidden: true }, dialog);

  let resolve: ((ok: boolean) => void) | null = null;
  let returnFocus: HTMLElement | null = null;

  const close = (ok: boolean) => {
    if (!resolve) return;
    el.hidden = true;
    const done = resolve;
    resolve = null;
    returnFocus?.focus();
    done(ok);
  };

  cancelBtn.addEventListener('click', () => close(false));
  confirmBtn.addEventListener('click', () => close(true));
  el.addEventListener('mousedown', (e) => {
    if (e.target === el) close(false);
  });
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      close(false);
    } else if (e.key === 'Tab') {
      // Keep focus on the two buttons while open.
      e.preventDefault();
      (document.activeElement === cancelBtn ? confirmBtn : cancelBtn).focus();
    }
  });

  return {
    el,
    ask(opts) {
      close(false); // a pending question is answered "no"
      title.textContent = opts.title;
      message.textContent = opts.message;
      confirmBtn.textContent = opts.confirmLabel;
      cancelBtn.textContent = opts.cancelLabel ?? 'Cancel';
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      el.hidden = false;
      cancelBtn.focus(); // the safe choice is the default
      return new Promise<boolean>((r) => (resolve = r));
    },
    isOpen: () => !el.hidden,
  };
}
