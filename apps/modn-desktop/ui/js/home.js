import { listGames } from './api.js';
import { messageFor } from './messages.js';

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

function installationLink(gameId, path) {
  const link = element('a', 'modn-link', path);
  link.href = `#/game/${encodeURIComponent(gameId)}?path=${encodeURIComponent(path)}`;
  link.setAttribute('aria-label', `Open ${gameId} at ${path}`);
  link.dataset.testid = 'open-installation';
  link.dataset.gameId = gameId;
  link.dataset.path = path;
  return link;
}

function gameCard(gameId, paths) {
  const card = element('article', 'modn-card modn-game-card');
  card.dataset.testid = 'game-card';
  card.dataset.gameId = gameId;
  card.append(element('h2', 'modn-game-card__title', gameId));
  const list = element('ul', 'modn-list');
  for (const path of paths) {
    const item = element('li', 'modn-row');
    const text = element('div', 'modn-row__text');
    text.append(installationLink(gameId, path));
    item.append(text);
    list.append(item);
  }
  card.append(list);
  return card;
}

function emptyState() {
  const card = element('div', 'modn-card modn-card--empty');
  card.dataset.testid = 'home-empty';
  card.append(element('h2', undefined, 'No games registered'));
  card.append(
    element(
      'p',
      'modn-muted',
      'Register an installation with the modn CLI, then reopen this screen.',
    ),
  );
  return card;
}

/**
 * Home view: games and their installations.
 * @param {HTMLElement} root
 * @param {{path: string}} ctx
 */
export async function render(root, ctx) {
  void ctx;
  const view = element('div', 'modn-view');
  const header = element('header', 'modn-view__header');
  const titles = element('div', 'modn-row__text');
  titles.append(element('h1', undefined, 'Games'));
  titles.append(element('p', 'modn-muted', 'Installations registered with Modn.'));
  header.append(titles);
  const status = element('p', 'modn-status');
  status.setAttribute('role', 'status');
  const grid = element('div', 'modn-grid');
  grid.dataset.testid = 'game-grid';
  view.append(header, status, grid);
  root.append(view);
  status.textContent = 'Loading games…';
  try {
    const games = await listGames();
    status.textContent = '';
    if (games.length === 0) {
      grid.append(emptyState());
      return;
    }
    const grouped = new Map();
    for (const game of games) {
      const paths = grouped.get(game.game_id) || [];
      paths.push(game.path);
      grouped.set(game.game_id, paths);
    }
    for (const [gameId, paths] of grouped) {
      grid.append(gameCard(gameId, paths));
    }
  } catch (error) {
    status.textContent = messageFor(error);
    status.classList.add('modn-status--error');
  }
}
