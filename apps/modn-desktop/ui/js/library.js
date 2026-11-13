import { installArchive, listInstallations, listPackages, newRunId, pickArchive, uninstall } from './api.js';
import { messageFor } from './messages.js';
import { showToast } from './toast.js';

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

function statusLine(message, isError = false) {
  const status = element('p', isError ? 'modn-status modn-status--error' : 'modn-status', message);
  status.setAttribute('role', 'status');
  return status;
}

function backLink(gameId, path) {
  const href = path ? `#/game/${encodeURIComponent(gameId)}?path=${encodeURIComponent(path)}` : '#/';
  const link = element('a', 'modn-link', 'Back to games');
  link.href = href;
  link.dataset.testid = 'back-to-home';
  return link;
}

/**
 * Library view: owned packages for one installation, one-click install and
 * uninstall.
 * @param {HTMLElement} root
 * @param {{gameId: string, path: string}} ctx
 */
export async function render(root, ctx) {
  const view = element('div', 'modn-view');
  const header = element('header', 'modn-view__header');
  const titles = element('div', 'modn-row__text');
  titles.append(element('h1', undefined, ctx.gameId));
  titles.append(element('p', 'modn-muted', ctx.path || 'No installation selected.'));
  const actions = element('div', 'modn-actions');
  const install = element('button', 'modn-button', 'Install archive');
  install.type = 'button';
  install.dataset.testid = 'install-archive';
  install.setAttribute('aria-label', `Install a local archive in ${ctx.gameId}`);
  actions.append(install);
  header.append(titles, actions);
  const status = statusLine('');
  const list = element('ul', 'modn-list modn-package-list');
  list.dataset.testid = 'package-list';
  view.append(header, status, list, backLink(ctx.gameId, ctx.path));
  root.append(view);

  const refresh = async () => {
    list.replaceChildren();
    if (!ctx.path) {
      await renderInstallations(list);
      return;
    }
    try {
      const packages = await listPackages({ gameId: ctx.gameId, path: ctx.path });
      if (packages.length === 0) {
        const empty = element('li', 'modn-card modn-card--empty');
        empty.append(element('p', 'modn-muted', 'No managed packages in this installation.'));
        list.append(empty);
        return;
      }
      for (const pkg of packages) {
        list.append(packageRow(pkg));
      }
    } catch (error) {
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
    }
  };

  const packageRow = (pkg) => {
    const name = pkg.mod_id || pkg.id;
    const item = element('li', 'modn-row modn-package');
    item.dataset.testid = 'package-row';
    item.dataset.packageId = pkg.id;
    const text = element('div', 'modn-row__text');
    text.append(element('span', 'modn-package__name', name));
    text.append(element('span', 'modn-muted', `${pkg.id} · ${pkg.files.length} owned files`));
    const button = element('button', 'modn-button modn-button--danger', 'Uninstall');
    button.type = 'button';
    button.dataset.testid = 'uninstall-package';
    button.dataset.packageId = pkg.id;
    button.setAttribute('aria-label', `Uninstall ${name}`);
    button.addEventListener('click', async () => {
      button.disabled = true;
      status.classList.remove('modn-status--error');
      status.textContent = `Uninstalling ${name}…`;
      try {
        await uninstall({
          gameId: ctx.gameId,
          path: ctx.path,
          packageId: pkg.id,
          runId: newRunId(),
        });
        showToast(`Uninstalled ${name}.`);
        status.textContent = '';
        await refresh();
      } catch (error) {
        status.textContent = messageFor(error);
        status.classList.add('modn-status--error');
        button.disabled = false;
      }
    });
    item.append(text, button);
    return item;
  };

  const renderInstallations = async (target) => {
    try {
      const installations = await listInstallations({ gameId: ctx.gameId });
      if (installations.length === 0) {
        target.append(element('li', 'modn-muted', 'This game is not registered.'));
        return;
      }
      for (const game of installations) {
        const item = element('li', 'modn-row');
        const link = element('a', 'modn-link', game.path);
        link.href = `#/game/${encodeURIComponent(ctx.gameId)}?path=${encodeURIComponent(game.path)}`;
        link.dataset.testid = 'open-installation';
        item.append(link);
        target.append(item);
      }
    } catch (error) {
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
    }
  };

  install.addEventListener('click', async () => {
    if (!ctx.path) {
      status.textContent = 'Choose an installation first.';
      status.classList.add('modn-status--error');
      return;
    }
    install.disabled = true;
    status.classList.remove('modn-status--error');
    status.textContent = 'Choose an archive…';
    try {
      const archive = await pickArchive();
      if (archive === null) {
        status.textContent = '';
        return;
      }
      status.textContent = 'Installing…';
      await installArchive({
        gameId: ctx.gameId,
        path: ctx.path,
        archive,
        allowNativeInjector: false,
        allowExecutable: false,
        runId: newRunId(),
      });
      showToast('Installed.');
      status.textContent = '';
      await refresh();
    } catch (error) {
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
      showToast(messageFor(error));
    } finally {
      install.disabled = false;
    }
  });

  await refresh();
}
