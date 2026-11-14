import { history, undo } from '../api.js';
import { messageFor } from '../messages.js';
import { createOperationButton } from '../op-button.js';
import { element } from './dom.js';
import { undoHooks } from './tile.js';

const KINDS = {
  install: 'Installed',
  uninstall: 'Uninstalled',
  update: 'Updated',
  undo: 'Undid',
};

/**
 * @param {Object} entry
 * @returns {string}
 */
function entryLabel(entry) {
  const kind = KINDS[entry.kind] || 'Changed';
  const names = Array.isArray(entry.packages)
    ? entry.packages.map((pkg) => pkg.mod_id).filter(Boolean)
    : [];
  return names.length > 0 ? `${kind} ${names.join(', ')}` : kind;
}

/**
 * Recent menu in the Library header: newest operations first, each with Undo.
 * @param {{gameId: string, onChanged: () => void|Promise<void>}} options
 * @returns {HTMLElement}
 */
export function createRecentMenu({ gameId, onChanged }) {
  const hooks = undoHooks(gameId);
  const wrapper = element('div', 'modn-recent');
  const button = element('button', 'modn-button modn-button--quiet', 'Recent');
  button.type = 'button';
  button.dataset.testid = 'library-recent';
  button.setAttribute('aria-haspopup', 'menu');
  button.setAttribute('aria-expanded', 'false');
  const menu = element('div', 'modn-recent__menu');
  menu.setAttribute('role', 'menu');
  menu.setAttribute('aria-label', 'Recent operations');
  menu.hidden = true;
  wrapper.append(button, menu);

  async function load() {
    menu.replaceChildren();
    menu.append(element('p', 'modn-muted', 'Loading operations…'));
    let entries;
    try {
      entries = await history({ gameId });
    } catch (error) {
      menu.replaceChildren(element('p', 'modn-status modn-status--error', messageFor(error)));
      return;
    }
    menu.replaceChildren();
    if (entries.length === 0) {
      menu.append(element('p', 'modn-muted', 'No operations yet.'));
      return;
    }
    for (const entry of entries) {
      const item = element('div', 'modn-recent__item');
      const row = element('div', 'modn-recent__entry');
      const askHost = element('div', 'modn-recent__ask');
      row.append(element('span', 'modn-recent__text', entryLabel(entry)));
      row.append(
        createOperationButton({
          ...hooks,
          label: 'Undo',
          action: 'Undo',
          testId: 'recent-undo',
          ariaLabel: `Undo ${entryLabel(entry)}`,
          askHost,
          run: ({ runId, approvals }) =>
            undo({
              operationId: entry.id,
              runId,
              allowNativeInjector: approvals.native,
              allowExecutables: approvals.executables,
            }),
          onSettled: async () => {
            await load();
            await onChanged();
          },
        }).element,
      );
      item.append(row, askHost);
      menu.append(item);
    }
  }

  function close() {
    menu.hidden = true;
    button.setAttribute('aria-expanded', 'false');
  }

  function open() {
    menu.hidden = false;
    button.setAttribute('aria-expanded', 'true');
    void load();
  }

  button.addEventListener('click', () => {
    if (menu.hidden) {
      open();
    } else {
      close();
    }
  });

  function onDocumentClick(event) {
    if (!wrapper.isConnected) {
      document.removeEventListener('click', onDocumentClick);
      return;
    }
    if (!menu.hidden && event.target instanceof Node && !wrapper.contains(event.target)) {
      close();
    }
  }
  document.addEventListener('click', onDocumentClick);

  return wrapper;
}
