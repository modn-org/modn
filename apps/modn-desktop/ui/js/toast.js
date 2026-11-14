const TOAST_TIMEOUT = 8000;

/**
 * Shows a status toast for eight seconds.
 * @param {string} message
 * @param {{actionLabel?: string, onAction?: () => void}} [options]
 */
export function showToast(message, { actionLabel, onAction } = {}) {
  let host = document.getElementById('modn-toasts');
  if (!host) {
    host = document.createElement('div');
    host.id = 'modn-toasts';
    host.className = 'modn-toasts';
    host.setAttribute('aria-live', 'polite');
    document.body.append(host);
  }
  const toast = document.createElement('div');
  toast.className = 'modn-toast';
  toast.setAttribute('role', 'status');
  toast.dataset.testid = 'toast';
  const text = document.createElement('span');
  text.className = 'modn-toast__message';
  text.textContent = message;
  toast.append(text);
  const timeout = window.setTimeout(() => toast.remove(), TOAST_TIMEOUT);
  if (actionLabel && onAction) {
    const action = document.createElement('button');
    action.type = 'button';
    action.className = 'modn-toast__action';
    action.textContent = actionLabel;
    action.dataset.testid = 'toast-action';
    action.addEventListener('click', () => {
      window.clearTimeout(timeout);
      toast.remove();
      onAction();
    });
    toast.append(action);
  }
  host.append(toast);
}
