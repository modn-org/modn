import { history, undo } from '../api.js';
import { messageFor } from '../messages.js';
import { createOperationButton } from '../op-button.js';
import { element } from './dom.js';
import { formatWhen, historyForMod, undoLabel } from './model.js';

/**
 * Undo plumbing shared with the operation buttons: the undo command itself.
 * @param {string} gameId
 * @returns {{runUndo: Function}}
 */
export function undoHooks(gameId) {
  return {
    runUndo: ({ runId, operationId, approvals }) =>
      undo({
        operationId,
        runId,
        allowNativeInjector: approvals.native,
        allowExecutables: approvals.executables,
      }),
  };
}

/**
 * Operation history of one mod, newest first, each row with Undo.
 * @param {{gameId: string, modId: string, path: string}} ctx
 * @param {{reload: () => Promise<void>|void}} options
 * @returns {{element: HTMLElement, load: (model: Object) => Promise<void>}}
 */
export function createHistorySection(ctx, { reload }) {
  const section = element('section', 'modn-card modn-details__history');
  section.dataset.testid = 'details-history';
  section.append(element('h2', 'modn-details__heading', 'History'));
  const status = element('p', 'modn-status');
  status.setAttribute('role', 'status');
  const list = element('ul', 'modn-history');
  list.setAttribute('aria-label', 'Operation history');
  section.append(status, list);

  const hooks = undoHooks(ctx.gameId);

  /** @param {string} message */
  function showError(message) {
    status.textContent = message;
    status.classList.add('modn-status--error');
  }

  /**
   * @param {Object} entry
   * @returns {HTMLLIElement}
   */
  function rowOf(entry) {
    const row = element('li', 'modn-row modn-history__entry');
    row.dataset.operationId = typeof entry.id === 'string' ? entry.id : '';
    const text = element('div', 'modn-row__text');
    const label = undoLabel(entry);
    text.append(element('p', 'modn-history__label', label));
    const when = formatWhen(entry.recorded_at);
    if (when) {
      text.append(element('p', 'modn-muted', when));
    }
    const askHost = element('div', 'modn-history__ask');
    const action = createOperationButton({
      ...hooks,
      label: 'Undo',
      action: 'Undo',
      testId: 'history-undo',
      ariaLabel: `Undo ${label}`,
      askHost,
      run: ({ runId, approvals }) =>
        undo({
          operationId: entry.id,
          runId,
          allowNativeInjector: approvals.native,
          allowExecutables: approvals.executables,
        }),
      onSettled: reload,
      onError: showError,
    });
    row.append(text, action.element, askHost);
    return row;
  }

  /**
   * @param {Object[]} entries
   */
  function renderEntries(entries) {
    list.replaceChildren();
    if (entries.length === 0) {
      const empty = element('li', 'modn-muted', 'No operations for this mod yet.');
      empty.dataset.testid = 'details-history-empty';
      list.append(empty);
      return;
    }
    for (const entry of entries) {
      list.append(rowOf(entry));
    }
  }

  /**
   * @param {{installed: Object|null}} model
   */
  async function load(model) {
    status.classList.remove('modn-status--error');
    status.textContent = 'Loading history…';
    try {
      const entries = await history({ gameId: ctx.gameId });
      const packageId =
        model && model.installed && typeof model.installed.id === 'string'
          ? model.installed.id
          : '';
      renderEntries(historyForMod(entries, ctx.modId, packageId));
      status.textContent = '';
    } catch (error) {
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
    }
  }

  return { element: section, load };
}
