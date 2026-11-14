import { cancelRun, newRunId, onProgress } from './api.js';
import { approvalsFrom } from './library/approvals.js';
import { renderAsk } from './library/ask.js';
import { operationIdOf } from './library/undo-target.js';
import { messageFor } from './messages.js';
import { showToast } from './toast.js';

/** Codes that describe a blocked plan: the button shows the reason instead. */
const BLOCKED_CODES = new Set([
  'PLAN_BLOCKED',
  'CONFLICT',
  'PACKAGE_NEEDED',
  'PACKAGE_PINNED',
  'DEPENDENCY_MISSING',
  'ALREADY_INSTALLED',
  'ARCHIVE_INVALID',
  'ARCHIVE_REQUIRED',
  'DIGEST_MISMATCH',
  'MOD_NOT_FOUND',
  'AMBIGUOUS_TARGET',
  'PENDING_TRANSACTION',
]);

const NO_APPROVAL = { native: false, executables: [] };

/** @param {unknown} error @returns {string} */
function codeOf(error) {
  if (error && typeof error === 'object') {
    const value = error.code ?? error.error;
    return typeof value === 'string' ? value : '';
  }
  return typeof error === 'string' ? error : '';
}

/** Shows a toast whose action is labelled Undo and carries the verifier hook. */
function showUndoToast(message, onUndo) {
  showToast(message, { actionLabel: 'Undo', onAction: onUndo });
  const host = document.getElementById('modn-toasts');
  const action = host && host.lastElementChild
    ? host.lastElementChild.querySelector('.modn-toast__action')
    : null;
  if (action) {
    action.dataset.testid = 'toast-undo';
  }
}

/**
 * Creates a one-click operation button with cancel, progress, Finishing...,
 * blocked reasons and a remembered approval ask. Mod details reuses it.
 *
 * @param {{
 *   label: string,
 *   action: string,
 *   run: (args: {runId: string, params: Object,
 *     approvals: {native: boolean, executables: string[]},
 *     operationId?: string}) => Promise<unknown>,
 *   prepare?: () => Promise<Object|null>,
 *   plan?: () => Promise<Object|null>,
 *   askHost?: HTMLElement|null,
 *   runUndo?: (args: {runId: string, operationId: string,
 *     approvals: {native: boolean, executables: string[]}}) => Promise<unknown>,
 *   launchUndo?: ((operationId: string) => void)|null,
 *   onSettled?: () => void|Promise<void>,
 *   onError?: (message: string) => void,
 *   onBlocked?: () => void,
 *   onDeclined?: () => void,
 *   testId?: string,
 *   ariaLabel?: string,
 *   disabled?: boolean,
 * }} options
 * @returns {{
 *   element: HTMLElement,
 *   button: HTMLButtonElement,
 *   start: (overrides?: Object) => Promise<void>,
 *   setBlocked: (message: string) => void,
 *   reset: () => void,
 *   destroy: () => void,
 * }}
 */
export function createOperationButton(options) {
  const {
    label,
    action,
    run,
    prepare = null,
    plan = null,
    askHost = null,
    runUndo = null,
    launchUndo = null,
    onSettled = null,
    onError = null,
    onBlocked = null,
    onDeclined = null,
    testId = 'op-button',
    ariaLabel = label,
    disabled = false,
  } = options;

  const wrapper = document.createElement('span');
  wrapper.className = 'modn-op';
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'modn-button modn-op__button';
  button.dataset.testid = testId;
  button.dataset.state = 'idle';
  button.textContent = label;
  button.setAttribute('aria-label', ariaLabel);
  button.disabled = disabled;
  const progress = document.createElement('span');
  progress.className = 'modn-op__progress';
  progress.hidden = true;
  const bar = document.createElement('span');
  bar.className = 'modn-op__bar';
  progress.append(bar);
  wrapper.append(button, progress);

  let runId = null;
  let unlisten = null;
  let generation = 0;
  let approvals = { ...NO_APPROVAL };
  let lastParams = {};
  let destroyed = false;
  let cancelRequested = false;
  let writeStarted = false;
  let settled = false;

  /** @param {number} percent */
  function setProgress(percent) {
    bar.style.setProperty('--op-progress', `${Math.min(100, Math.max(0, percent))}%`);
  }

  /** @param {string} text */
  function setIdle(text = label) {
    button.dataset.state = 'idle';
    button.disabled = disabled;
    button.textContent = text;
    button.removeAttribute('title');
    button.setAttribute('aria-label', ariaLabel);
    progress.hidden = true;
    setProgress(0);
  }

  /** @param {number} id @param {Object} event */
  function handleEvent(id, event) {
    if (destroyed || id !== runId) {
      return;
    }
    const phase = event && typeof event.phase === 'string' ? event.phase : '';
    if (phase === 'commit') {
      button.dataset.state = 'finishing';
      button.disabled = true;
      button.textContent = 'Finishing...';
      setProgress(100);
      return;
    }
    const bytes = event && Number.isFinite(event.bytes) ? event.bytes : null;
    const total = event && Number.isFinite(event.total) ? event.total : null;
    if (phase === 'download' && bytes !== null && total !== null && total > 0) {
      const percent = Math.round((bytes / total) * 100);
      setProgress(percent);
      button.textContent = `Cancel · ${percent} %`;
      return;
    }
    if (phase) {
      button.textContent = `Cancel · ${phase}`;
    }
  }

  /**
   * Records the click at once. Before the write starts, start() skips it and
   * reports Cancelled. Once the write is running, the backend is told.
   */
  async function requestCancel() {
    if (!runId || settled) {
      return;
    }
    const id = runId;
    cancelRequested = true;
    button.disabled = true;
    button.textContent = 'Cancelling…';
    if (!writeStarted) {
      return;
    }
    try {
      await cancelRun({ runId: id });
    } catch {
      if (id === runId && !settled) {
        button.disabled = false;
        button.textContent = 'Cancel';
      }
    }
  }

  /**
   * Reverses one committed operation by the ID its toast bound.
   * @param {string} operationId
   */
  async function undoLatest(operationId) {
    if (!runUndo || runId || !operationId) {
      return;
    }
    await start({
      run: runUndo,
      label: 'Undo',
      action: 'Undo',
      operationId,
      params: {},
      approvals: { ...NO_APPROVAL },
    });
  }

  /**
   * @param {Object} [overrides]
   */
  async function start(overrides = {}) {
    if (runId || destroyed) {
      return;
    }
    const active = {
      run: overrides.run || run,
      idleLabel: overrides.label || label,
      actionLabel: overrides.action || action,
      operationId: overrides.operationId,
    };
    const id = newRunId();
    const generationAtStart = ++generation;
    runId = id;
    cancelRequested = false;
    writeStarted = false;
    settled = false;
    button.dataset.state = 'running';
    button.disabled = true;
    button.textContent = 'Preparing…';
    button.setAttribute('aria-label', `Preparing ${active.idleLabel}`);
    progress.hidden = false;
    setProgress(0);
    try {
      let params = overrides.params;
      if (params === undefined) {
        if (!prepare) {
          params = {};
        } else {
          params = await prepare();
          if (generationAtStart !== generation || destroyed) {
            return;
          }
          if (params === null) {
            setIdle(active.idleLabel);
            return;
          }
        }
      }
      lastParams = params;
      approvals = overrides.approvals ? { ...overrides.approvals } : { ...NO_APPROVAL };
      button.disabled = false;
      button.textContent = 'Cancel';
      button.setAttribute('aria-label', `Cancel ${active.idleLabel}`);
      const off = await onProgress(id, (event) => handleEvent(id, event));
      if (generationAtStart !== generation || destroyed) {
        if (typeof off === 'function') {
          try {
            off();
          } catch {
            // The event listener is already gone.
          }
        }
        return;
      }
      unlisten = off;
      if (cancelRequested) {
        settled = true;
        setIdle(active.idleLabel);
        showToast('Cancelled. Nothing changed.');
        if (onSettled) {
          await onSettled();
        }
        return;
      }
      writeStarted = true;
      const result = await active.run({
        runId: id,
        params,
        approvals: { ...approvals },
        operationId: active.operationId,
      });
      if (generationAtStart !== generation || destroyed) {
        return;
      }
      settled = true;
      const committedId = operationIdOf(result);
      setIdle(active.idleLabel);
      if (committedId) {
        showUndoToast(`${active.actionLabel} done`, () => {
          if (launchUndo) {
            launchUndo(committedId);
          } else {
            void undoLatest(committedId);
          }
        });
      } else {
        showToast(`${active.actionLabel} done`);
      }
      if (onSettled) {
        await onSettled();
      }
    } catch (error) {
      if (generationAtStart !== generation || destroyed) {
        return;
      }
      settled = true;
      const code = codeOf(error);
      if (code === 'CANCELLED') {
        setIdle(active.idleLabel);
        showToast('Cancelled. Nothing changed.');
        if (onSettled) {
          await onSettled();
        }
      } else if (code === 'BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT') {
        setIdle(active.idleLabel);
        showToast(messageFor(code));
        if (onSettled) {
          await onSettled();
        }
      } else if (code === 'APPROVAL_REQUIRED' || code === 'NATIVE_INJECTOR_NOT_ALLOWED') {
        approvals = await approvalsFrom(error, code, askHost && !active.operationId ? plan : null);
        if (generationAtStart !== generation || destroyed) {
          return;
        }
        setIdle(active.idleLabel);
        if (askHost) {
          const uncovered = { ...approvals };
          renderAsk(
            askHost,
            uncovered,
            () => {
              void start({
                run: active.run,
                label: active.idleLabel,
                action: active.actionLabel,
                operationId: active.operationId,
                params: lastParams,
                approvals: uncovered,
              });
            },
            () => {
              setIdle(active.idleLabel);
              if (onDeclined) {
                onDeclined();
              }
            },
          );
        } else {
          showToast(messageFor(error));
        }
      } else if (BLOCKED_CODES.has(code)) {
        const reason = messageFor(code);
        button.dataset.state = 'blocked';
        button.disabled = true;
        button.textContent = reason;
        button.title = reason;
        button.setAttribute('aria-label', reason);
        progress.hidden = true;
        setProgress(0);
        if (onBlocked) {
          onBlocked();
        }
      } else {
        setIdle(active.idleLabel);
        showToast(messageFor(error));
        if (onError) {
          onError(messageFor(error));
        }
      }
    } finally {
      if (runId === id) {
        runId = null;
        cancelRequested = false;
        writeStarted = false;
        settled = false;
      }
      if (unlisten) {
        const off = unlisten;
        unlisten = null;
        try {
          off();
        } catch {
          // The event listener is already gone.
        }
      }
    }
  }

  button.addEventListener('click', () => {
    if (runId) {
      void requestCancel();
    } else {
      void start();
    }
  });

  return {
    element: wrapper,
    button,
    start,
    /** @param {string} reason */
    setBlocked(reason) {
      button.dataset.state = 'blocked';
      button.disabled = true;
      button.textContent = reason;
      button.title = reason;
      button.setAttribute('aria-label', reason);
      progress.hidden = true;
    },
    reset() {
      setIdle();
    },
    destroy() {
      destroyed = true;
      generation += 1;
      runId = null;
      if (unlisten) {
        try {
          unlisten();
        } catch {
          // The event listener is already gone.
        }
        unlisten = null;
      }
      if (askHost) {
        askHost.replaceChildren();
      }
      wrapper.remove();
    },
  };
}
