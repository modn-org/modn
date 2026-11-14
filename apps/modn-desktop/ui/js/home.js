import { activateGame, addGame, removeGame } from './api.js';
import { messageFor } from './messages.js';
import { buildCard, clearCardError, setCardBusy, showCardError } from './home/card.js';
import { gameLabel, loadHome, supportedGameIds, supportedSentence } from './home/data.js';
import { button, element, setStatus, statusLine } from './home/dom.js';
import { createAddMenu } from './home/menu.js';

const SCAN_MESSAGE = 'Scanning this PC for supported games…';

/**
 * @typedef {Object} HomeEntry
 * @property {string} gameId
 * @property {string} path
 * @property {'registered'|'detected'} state
 * @property {string} store
 * @property {boolean} [canRemove]
 */

/**
 * Builds the library route for one registered installation.
 * @param {HomeEntry} entry
 * @returns {string}
 */
function installationRoute(entry) {
  return `#/game/${encodeURIComponent(entry.gameId)}?path=${encodeURIComponent(entry.path)}`;
}

/**
 * Explains that this PC holds no registered and no detected game. The Scan and
 * Add controls stay in the toolbar, so this card only names them.
 * @returns {HTMLElement}
 */
function emptyState() {
  const card = element('div', 'modn-card modn-card--empty modn-home__empty');
  card.dataset.testid = 'home-empty';
  card.append(element('h2', undefined, 'No supported games found'));
  card.append(
    element(
      'p',
      'modn-muted',
      'Modn found no registered installation and no supported game on this PC.',
    ),
  );
  card.append(
    element('p', 'modn-muted', `Supported games: ${supportedSentence(supportedGameIds([]))}.`),
  );
  card.append(
    element(
      'p',
      'modn-muted',
      'Select Scan to search this PC again, or Add to choose a game folder.',
    ),
  );
  return card;
}

/**
 * Home view: registered games, detected games, and the actions that change them.
 * @param {HTMLElement} root
 */
export async function render(root) {
  const view = element('div', 'modn-view modn-home');
  const header = element('header', 'modn-view__header');
  const titles = element('div', 'modn-row__text');
  titles.append(element('h1', undefined, 'Games'));
  titles.append(
    element('p', 'modn-muted', 'Games Modn has registered, and supported games on this PC.'),
  );
  const actions = element('div', 'modn-actions modn-home__actions');
  const scan = button(
    'Scan',
    'home-scan',
    'Scan this PC for supported games',
    'modn-button modn-button--secondary',
  );
  const add = button('Add', 'home-add', 'Add a game folder to Modn', 'modn-button modn-button--quiet');
  actions.append(scan, add);
  header.append(titles, actions);

  const status = statusLine();
  const grid = element('div', 'modn-grid modn-home__grid');
  grid.dataset.testid = 'game-grid';
  view.append(header, status, grid);
  root.append(view);

  /** @type {HomeEntry[]} */
  let entries = [];
  let generation = 0;

  const menu = createAddMenu({
    anchor: add,
    container: actions,
    listGameIds: () => supportedGameIds(entries),
    onPick: (gameId) => {
      void addGameFlow(gameId);
    },
  });

  const handlers = {
    /** @param {HomeEntry} entry */
    open(entry) {
      menu.close();
      window.location.hash = installationRoute(entry);
    },
    /** @param {HomeEntry} entry @param {HTMLElement} card */
    activate(entry, card) {
      void activateFlow(entry, card);
    },
    /** @param {HomeEntry} entry @param {HTMLElement} card @param {HTMLButtonElement} control */
    remove(entry, card, control) {
      void removeFlow(entry, card, control);
    },
  };

  /**
   * Reloads registration and detection, then rebuilds the grid.
   * @param {string} message shown while the reload runs
   * @param {string} [notice] shown when the reload succeeds
   * @returns {Promise<void>}
   */
  async function refresh(message, notice = '') {
    menu.close();
    const current = ++generation;
    setStatus(status, message);
    grid.setAttribute('aria-busy', 'true');
    const state = await loadHome();
    if (current !== generation) {
      return;
    }
    grid.removeAttribute('aria-busy');
    entries = state.entries;
    grid.replaceChildren();
    if (state.listError) {
      setStatus(status, messageFor(state.listError), true);
      return;
    }
    if (entries.length === 0) {
      grid.append(emptyState());
    } else {
      for (const entry of entries) {
        grid.append(buildCard(entry, handlers));
      }
    }
    setStatus(
      status,
      state.detectError ? messageFor(state.detectError) : notice,
      Boolean(state.detectError),
    );
  }

  /**
   * Registers one detected installation and rebuilds the grid.
   * @param {HomeEntry} entry
   * @param {HTMLElement} card
   * @returns {Promise<void>}
   */
  async function activateFlow(entry, card) {
    setCardBusy(card, true);
    clearCardError(card);
    try {
      await activateGame({ gameId: entry.gameId, path: entry.path });
    } catch (error) {
      setCardBusy(card, false);
      showCardError(card, messageFor(error));
      return;
    }
    await refresh('', `Added ${gameLabel(entry.gameId)} to Modn.`);
  }

  /**
   * Unregisters one installation with no managed package.
   * @param {HomeEntry} entry
   * @param {HTMLElement} card
   * @param {HTMLButtonElement} control
   * @returns {Promise<void>}
   */
  async function removeFlow(entry, card, control) {
    control.disabled = true;
    clearCardError(card);
    try {
      const removed = await removeGame({ gameId: entry.gameId, path: entry.path });
      if (!removed) {
        control.disabled = false;
        return;
      }
    } catch (error) {
      control.disabled = false;
      showCardError(card, messageFor(error));
      return;
    }
    await refresh('', `Removed ${gameLabel(entry.gameId)} from Modn.`);
  }

  /**
   * Opens the native folder picker for one game and registers the choice.
   * @param {string} gameId
   * @returns {Promise<void>}
   */
  async function addGameFlow(gameId) {
    add.disabled = true;
    try {
      const added = await addGame({ gameId });
      if (added) {
        await refresh('', `Added ${gameLabel(gameId)} to Modn.`);
      }
    } catch (error) {
      setStatus(status, messageFor(error), true);
    } finally {
      add.disabled = false;
    }
  }

  scan.addEventListener('click', () => {
    scan.disabled = true;
    void refresh(SCAN_MESSAGE).finally(() => {
      scan.disabled = false;
    });
  });

  await refresh(SCAN_MESSAGE);
}

