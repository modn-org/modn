import { catalog, installArchive, listInstallations, listPackages, pickArchive, play } from './api.js';
import { messageFor } from './messages.js';
import { createOperationButton } from './op-button.js';
import { showToast } from './toast.js';
import { mergeLibrary, visibleModels } from './library/data.js';
import { element } from './library/dom.js';
import { createRecentMenu } from './library/recent.js';
import { createTile, createUndoLauncher, undoHooks } from './library/tile.js';

const FILTERS = [
  { id: 'all', label: 'All', testId: 'filter-all' },
  { id: 'installed', label: 'Installed', testId: 'filter-installed' },
  { id: 'available', label: 'Available', testId: 'filter-available' },
];

/**
 * Library view: one game installation, its mod grid, filters, search and
 * one-click operations.
 * @param {HTMLElement} root
 * @param {{gameId: string, path: string}} ctx
 */
export async function render(root, ctx) {
  const state = {
    entries: [],
    packages: [],
    filter: 'all',
    query: '',
    loadToken: 0,
  };
  /** @type {Map<string, {button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>} */
  const operations = new Map();
  const view = element('div', 'modn-view modn-library');
  const header = element('header', 'modn-view__header modn-library__hero');
  const titles = element('div', 'modn-row__text');
  titles.append(element('h1', 'modn-library__title', ctx.gameId));
  titles.append(element('p', 'modn-muted', ctx.path || 'No installation selected.'));
  let installations = [];
  try {
    installations = await listInstallations({ gameId: ctx.gameId });
  } catch {
    installations = [];
  }
  if (installations.length > 1) {
    const field = element('label', 'modn-library__installation');
    field.append(element('span', 'modn-library__label', 'Installation'));
    const select = document.createElement('select');
    select.className = 'modn-select';
    select.dataset.testid = 'library-installation';
    select.setAttribute('aria-label', 'Installation');
    for (const game of installations) {
      const option = document.createElement('option');
      option.value = game.path;
      option.textContent = game.path;
      option.selected = game.path === ctx.path;
      select.append(option);
    }
    select.addEventListener('change', () => {
      window.location.hash =
        `#/game/${encodeURIComponent(ctx.gameId)}?path=${encodeURIComponent(select.value)}`;
    });
    field.append(select);
    titles.append(field);
  }
  const actions = element('div', 'modn-actions modn-library__actions');
  const playButton = element('button', 'modn-button', 'Play');
  playButton.type = 'button';
  playButton.dataset.testid = 'library-play';
  playButton.setAttribute('aria-label', `Play ${ctx.gameId}`);
  playButton.disabled = !ctx.path;
  playButton.addEventListener('click', async () => {
    if (!ctx.path) {
      return;
    }
    playButton.disabled = true;
    try {
      await play({ gameId: ctx.gameId, path: ctx.path });
    } catch (error) {
      showToast(messageFor(error));
    } finally {
      playButton.disabled = !ctx.path;
    }
  });

  const status = element('p', 'modn-status');
  status.setAttribute('role', 'status');
  const hooks = undoHooks(ctx.gameId);
  const archiveAsk = element('div', 'modn-library__ask');
  const undoHost = element('div', 'modn-library__ask');
  const installButton = createOperationButton({
    ...hooks,
    launchUndo: createUndoLauncher({ ...hooks, host: undoHost, onSettled: refresh }),
    label: 'Install archive',
    action: 'Install',
    testId: 'library-install-archive',
    ariaLabel: `Install a local archive in ${ctx.gameId}`,
    disabled: !ctx.path,
    askHost: archiveAsk,
    prepare: async () => {
      const archive = await pickArchive();
      return archive ? { archive } : null;
    },
    run: ({ runId, params, approvals }) =>
      installArchive({
        gameId: ctx.gameId,
        path: ctx.path,
        archive: params.archive,
        allowNativeInjector: approvals.native,
        allowExecutables: approvals.executables,
        runId,
      }),
    onSettled: refresh,
    onError(message) {
      status.textContent = message;
      status.classList.add('modn-status--error');
    },
  });
  actions.append(playButton, installButton.element);
  actions.append(createRecentMenu({ gameId: ctx.gameId, onChanged: refresh, undoHost }));
  header.append(titles, actions);

  const toolbar = element('div', 'modn-library__toolbar');
  const tabs = element('div', 'modn-tabs');
  tabs.setAttribute('role', 'tablist');
  tabs.setAttribute('aria-label', 'Filter mods');
  const tabButtons = new Map();
  for (const filter of FILTERS) {
    const tab = element('button', 'modn-tab', filter.label);
    tab.type = 'button';
    tab.setAttribute('role', 'tab');
    tab.dataset.testid = filter.testId;
    tab.id = `library-tab-${filter.id}`;
    tab.addEventListener('click', () => setFilter(filter.id));
    tabButtons.set(filter.id, tab);
    tabs.append(tab);
  }
  tabs.addEventListener('keydown', (event) => {
    const index = FILTERS.findIndex((filter) => filter.id === state.filter);
    let next = -1;
    if (event.key === 'ArrowRight') {
      next = (index + 1) % FILTERS.length;
    } else if (event.key === 'ArrowLeft') {
      next = (index - 1 + FILTERS.length) % FILTERS.length;
    } else if (event.key === 'Home') {
      next = 0;
    } else if (event.key === 'End') {
      next = FILTERS.length - 1;
    }
    if (next < 0) {
      return;
    }
    event.preventDefault();
    setFilter(FILTERS[next].id);
    tabButtons.get(FILTERS[next].id).focus();
  });
  const search = element('label', 'modn-library__search');
  search.append(element('span', 'modn-library__label', 'Search mods'));
  const searchInput = document.createElement('input');
  searchInput.type = 'search';
  searchInput.className = 'modn-input';
  searchInput.dataset.testid = 'library-search';
  searchInput.setAttribute('aria-label', 'Search mods');
  searchInput.placeholder = 'Search mods';
  searchInput.addEventListener('input', () => {
    state.query = searchInput.value.trim();
    renderGrid();
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => {
      void refresh();
    }, 250);
  });
  search.append(searchInput);
  toolbar.append(tabs, search);

  const grid = element('ul', 'modn-grid modn-library__grid');
  grid.dataset.testid = 'library-grid';
  grid.setAttribute('aria-label', 'Mods');
  const back = element('a', 'modn-link', 'Back to games');
  back.href = '#/';
  back.dataset.testid = 'back-to-home';

  view.append(header, status, archiveAsk, undoHost);

  if (!ctx.path) {
    status.textContent = 'Choose an installation first.';
    view.append(installationList(installations, ctx), back);
    root.append(view);
    return;
  }

  view.append(toolbar, grid, back);
  root.append(view);
  setFilter('all');

  let searchTimer = 0;

  function emptyTitle() {
    if (state.query) {
      return 'No mods match your search';
    }
    if (state.filter === 'installed') {
      return 'No installed mods';
    }
    if (state.filter === 'available') {
      return 'Everything is installed';
    }
    return 'No mods found';
  }

  function emptyText() {
    if (state.filter === 'installed' || state.filter === 'available') {
      return 'Switch the filter to see other mods in this installation.';
    }
    return 'The selected bucket lists no mods for this game.';
  }

  function renderGrid() {
    const models = mergeLibrary(state.entries, state.packages);
    const visible = visibleModels(models, state.filter, state.query);
    grid.replaceChildren();
    if (visible.length === 0) {
      const empty = element('li', 'modn-card modn-card--empty modn-library__empty');
      empty.dataset.testid = 'library-empty';
      empty.append(element('h2', undefined, emptyTitle()));
      empty.append(element('p', 'modn-muted', emptyText()));
      grid.append(empty);
      return;
    }
    for (const model of visible) {
      grid.append(createTile(model, ctx, { onChanged: refresh, operations, undoHost }));
    }
  }

  function setFilter(id) {
    state.filter = id;
    for (const [filterId, tab] of tabButtons) {
      const active = filterId === id;
      tab.classList.toggle('modn-tab--active', active);
      tab.setAttribute('aria-selected', active ? 'true' : 'false');
      tab.tabIndex = active ? 0 : -1;
    }
    renderGrid();
  }

  async function refresh() {
    const token = ++state.loadToken;
    status.classList.remove('modn-status--error');
    status.textContent = 'Loading mods…';
    try {
      const [entries, packages] = await Promise.all([
        catalog({ gameId: ctx.gameId, path: ctx.path, query: state.query || undefined }),
        listPackages({ gameId: ctx.gameId, path: ctx.path }),
      ]);
      if (token !== state.loadToken) {
        return;
      }
      state.entries = Array.isArray(entries) ? entries : [];
      state.packages = Array.isArray(packages) ? packages : [];
      status.textContent = '';
      renderGrid();
    } catch (error) {
      if (token !== state.loadToken) {
        return;
      }
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
    }
  }

  await refresh();
}

/**
 * Lists the installations of a game that has no selected path.
 * @param {Array<{path: string}>} installations
 * @param {{gameId: string}} ctx
 * @returns {HTMLUListElement}
 */
function installationList(installations, ctx) {
  const list = element('ul', 'modn-list');
  list.dataset.testid = 'installation-list';
  if (installations.length === 0) {
    list.append(element('li', 'modn-muted', 'This game is not registered.'));
    return list;
  }
  for (const game of installations) {
    const item = element('li', 'modn-row');
    const link = element('a', 'modn-link', game.path);
    link.href =
      `#/game/${encodeURIComponent(ctx.gameId)}?path=${encodeURIComponent(game.path)}`;
    link.dataset.testid = 'open-installation';
    item.append(link);
    list.append(item);
  }
  return list;
}
