import * as details from './details.js';
import * as home from './home.js';
import * as library from './library.js';
import { showToast } from './toast.js';
import { clearTheme, loadThemeForGame } from './theme/controller.js';
import { createThemePanel } from './theme/panel.js';

const mount = document.getElementById('modn-view');
const themePanel = createThemePanel(document.getElementById('modn-theme-host'));
let navigation = 0;
let activeGame;

function applyThemeFor(gameId) {
  if (gameId === activeGame) {
    return;
  }
  activeGame = gameId;
  themePanel.setGame(gameId);
  if (gameId === null) {
    clearTheme();
    themePanel.setTheme({ applied: false });
    return;
  }
  void loadThemeForGame(gameId).then((result) => {
    if (gameId !== activeGame) {
      return;
    }
    themePanel.setTheme(result);
  });
}

function parseRoute() {
  const raw = window.location.hash.replace(/^#/, '') || '/';
  const [pathPart, queryPart = ''] = raw.split('?');
  const segments = pathPart.split('/').filter(Boolean).map(decodeURIComponent);
  const params = new URLSearchParams(queryPart);
  const path = params.get('path') || '';
  if (segments.length === 0) {
    return { module: home, ctx: { view: 'home', path: '' } };
  }
  if (segments[0] === 'game' && segments.length === 2) {
    return { module: library, ctx: { view: 'library', gameId: segments[1], path } };
  }
  if (segments[0] === 'game' && segments[2] === 'mod' && segments.length === 4) {
    return {
      module: details,
      ctx: { view: 'details', gameId: segments[1], modId: segments[3], path },
    };
  }
  return { module: null, ctx: {} };
}

async function renderRoute() {
  const { module, ctx } = parseRoute();
  const current = ++navigation;
  applyThemeFor(ctx.gameId || null);
  const routeContainer = document.createElement('div');
  mount.replaceChildren(routeContainer);
  if (!module) {
    const message = document.createElement('p');
    message.className = 'modn-status modn-status--error';
    message.textContent = 'This screen does not exist.';
    routeContainer.append(message);
    mount.focus();
    return;
  }
  try {
    await module.render(routeContainer, ctx);
  } catch (error) {
    if (current !== navigation || !routeContainer.isConnected) {
      return;
    }
    showToast('This screen could not load.');
    throw error;
  } finally {
    if (current === navigation && routeContainer.isConnected) {
      mount.focus();
    }
  }
}

window.addEventListener('hashchange', () => {
  void renderRoute();
});

applyThemeFor(null);
void renderRoute();
