import * as details from './details.js';
import * as home from './home.js';
import * as library from './library.js';
import { showToast } from './toast.js';

const root = document.getElementById('modn-view');

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
  root.replaceChildren();
  if (!module) {
    const message = document.createElement('p');
    message.className = 'modn-status modn-status--error';
    message.textContent = 'This screen does not exist.';
    root.append(message);
    return;
  }
  try {
    await module.render(root, ctx);
  } catch (error) {
    showToast('This screen could not load.');
    throw error;
  } finally {
    root.focus();
  }
}

window.addEventListener('hashchange', () => {
  void renderRoute();
});

void renderRoute();
