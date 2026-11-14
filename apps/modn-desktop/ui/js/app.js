import * as details from './details.js';
import * as home from './home.js';
import * as library from './library.js';
import { showToast } from './toast.js';

const mount = document.getElementById('modn-view');
let navigation = 0;

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
  // Each navigation owns a fresh container. A late view keeps writing into
  // its detached container, so it cannot replace the screen on display.
  const container = document.createElement('div');
  mount.replaceChildren(container);
  if (!module) {
    const message = document.createElement('p');
    message.className = 'modn-status modn-status--error';
    message.textContent = 'This screen does not exist.';
    container.append(message);
    mount.focus();
    return;
  }
  try {
    await module.render(container, ctx);
  } catch (error) {
    if (current !== navigation || !container.isConnected) {
      return;
    }
    showToast('This screen could not load.');
    throw error;
  } finally {
    if (current === navigation && container.isConnected) {
      mount.focus();
    }
  }
}

window.addEventListener('hashchange', () => {
  void renderRoute();
});

void renderRoute();
