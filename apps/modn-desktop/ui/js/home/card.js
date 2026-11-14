/**
 * One Home card: a game installation that Modn manages, or a detected game
 * installation that Modn does not manage yet.
 */

import { gameLabel } from './data.js';
import { element } from './dom.js';

/**
 * @typedef {Object} CardEntry
 * @property {string} gameId
 * @property {string} path
 * @property {'registered'|'detected'} state
 * @property {string} store
 * @property {boolean} [canRemove]
 */

/**
 * Builds the one or two letters shown on the cover panel.
 * @param {string} gameId
 * @returns {string}
 */
function initials(gameId) {
  const words = gameLabel(gameId).split(' ').filter(Boolean);
  return words
    .slice(0, 2)
    .map((word) => word.charAt(0).toUpperCase())
    .join('');
}

/**
 * Describes the card for assistive technology.
 * @param {CardEntry} entry
 * @returns {string}
 */
function cardLabel(entry) {
  const name = gameLabel(entry.gameId);
  if (entry.state === 'detected') {
    return `Add ${name} at ${entry.path} to Modn`;
  }
  return `Open ${name} at ${entry.path}`;
}

/**
 * Shows a short reason on one card.
 * @param {HTMLElement} card
 * @param {string} message
 */
export function showCardError(card, message) {
  if (!card) {
    return;
  }
  const error = card.querySelector('[data-testid="game-card-error"]');
  if (!error) {
    return;
  }
  error.textContent = message;
  error.hidden = false;
}

/**
 * Marks one card as running an action. Activation is ignored while busy.
 * @param {HTMLElement} card
 * @param {boolean} busy
 */
export function setCardBusy(card, busy) {
  if (!card) {
    return;
  }
  if (busy) {
    card.dataset.busy = 'true';
    card.setAttribute('aria-disabled', 'true');
  } else {
    delete card.dataset.busy;
    card.setAttribute('aria-disabled', 'false');
  }
}

/**
 * Clears any earlier reason from one card.
 * @param {HTMLElement} card
 */
export function clearCardError(card) {
  if (!card) {
    return;
  }
  const error = card.querySelector('[data-testid="game-card-error"]');
  if (error) {
    error.textContent = '';
    error.hidden = true;
  }
}

/**
 * @param {CardEntry} entry
 * @param {{open: (entry: CardEntry) => void,
 *   activate: (entry: CardEntry, card: HTMLElement) => void,
 *   remove: (entry: CardEntry, card: HTMLElement, control: HTMLButtonElement) => void}} handlers
 * @returns {HTMLElement}
 */
export function buildCard(entry, handlers) {
  const detected = entry.state === 'detected';
  const card = element('article', 'modn-game-card');
  card.setAttribute('role', 'button');
  card.tabIndex = 0;
  card.dataset.state = entry.state;
  card.dataset.testid = `game-card-${entry.gameId}`;
  card.dataset.gameId = entry.gameId;
  card.dataset.path = entry.path;
  card.setAttribute('aria-label', cardLabel(entry));
  card.setAttribute('aria-disabled', 'false');

  const surface = element('div', 'modn-game-card__surface');

  const media = element('div', 'modn-game-card__media');
  media.setAttribute('aria-hidden', 'true');
  media.append(element('span', 'modn-game-card__initials', initials(entry.gameId)));

  const body = element('div', 'modn-game-card__body');
  body.append(element('span', 'modn-game-card__title', gameLabel(entry.gameId)));
  body.append(
    element('span', 'modn-game-card__meta', detected ? 'Detected on this PC' : 'In Modn'),
  );
  body.append(element('span', 'modn-game-card__path', entry.path));

  surface.append(
    media,
    body,
    element('span', 'modn-game-card__badge', detected ? `Not in Modn · ${entry.store}` : `Registered · ${entry.store}`),
  );

  if (detected) {
    surface.append(
      element('p', 'modn-muted modn-game-card__hint', 'Choose this card to add the game to Modn.'),
    );
  }

  const error = element('p', 'modn-status modn-status--error modn-game-card__error');
  error.dataset.testid = 'game-card-error';
  error.setAttribute('role', 'alert');
  error.hidden = true;

  const activate = () => {
    if (card.dataset.busy === 'true') {
      return;
    }
    clearCardError(card);
    if (detected) {
      handlers.activate(entry, card);
      return;
    }
    handlers.open(entry);
  };

  card.addEventListener('click', (event) => {
    if (event.target instanceof Element && event.target.closest('[data-testid="game-card-remove"]')) {
      return;
    }
    activate();
  });

  card.addEventListener('keydown', (event) => {
    if (event.target !== card) {
      return;
    }
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      activate();
    }
  });

  card.append(surface, error);

  if (!detected && entry.canRemove) {
    const remove = element('button', 'modn-button modn-button--quiet modn-game-card__remove', 'Remove from Modn');
    remove.type = 'button';
    remove.dataset.testid = 'game-card-remove';
    remove.dataset.gameId = entry.gameId;
    remove.dataset.path = entry.path;
    remove.setAttribute(
      'aria-label',
      `Remove ${gameLabel(entry.gameId)} at ${entry.path} from Modn`,
    );
    remove.addEventListener('click', (event) => {
      event.stopPropagation();
      handlers.remove(entry, card, remove);
    });
    card.append(remove);
  }

  return card;
}
