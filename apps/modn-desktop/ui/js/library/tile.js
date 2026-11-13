import { history, installMod, undo, uninstall, update } from '../api.js';
import { createOperationButton } from '../op-button.js';
import { element, detailsHref, initials } from './dom.js';

/**
 * Shared Undo plumbing: the newest history entry of the game, and the undo
 * command itself. Every operation button uses the same flow.
 * @param {string} gameId
 * @returns {{newestOperation: () => Promise<Object|null>, runUndo: Function}}
 */
export function undoHooks(gameId) {
  return {
    async newestOperation() {
      const entries = await history({ gameId });
      return entries.length > 0 ? entries[0] : null;
    },
    runUndo: ({ runId, operationId }) => undo({ operationId, runId }),
  };
}

/**
 * @param {import('./data.js').LibraryModel} model
 * @param {{gameId: string, path: string}} ctx
 * @param {{onChanged: () => void|Promise<void>}} options
 * @returns {HTMLLIElement}
 */
export function createTile(model, ctx, { onChanged }) {
  const hooks = undoHooks(ctx.gameId);
  const installed = model.installedPackage;
  const tile = element('li', 'modn-card modn-tile');
  tile.dataset.testid = `mod-tile-${model.modId}`;
  tile.dataset.modId = model.modId;
  tile.dataset.installed = installed ? 'true' : 'false';

  const art = element('div', 'modn-tile__art');
  art.setAttribute('aria-hidden', 'true');
  art.append(element('span', 'modn-tile__initials', initials(model.name)));

  const body = element('div', 'modn-tile__body');
  const name = element('a', 'modn-tile__name', model.name);
  name.href = detailsHref(ctx, model.modId);
  name.dataset.testid = 'mod-tile-details';
  name.setAttribute('aria-label', `Open details for ${model.name}`);

  const meta = [model.bucket, model.version].filter(Boolean).join(' · ');
  body.append(name);
  body.append(element('p', 'modn-muted', meta || 'Local archive'));

  const tags = element('div', 'modn-tile__tags');
  if (model.nativeInjector) {
    tags.append(element('span', 'modn-tag modn-tile__warning', 'Native code'));
  }
  if (installed) {
    tags.append(element('span', 'modn-tag', model.hasUpdate ? 'Update available' : 'Installed'));
  }
  if (tags.childElementCount > 0) {
    body.append(tags);
  }

  const state = element(
    'p',
    'modn-tile__state',
    installed ? (model.hasUpdate ? 'Update available' : 'Installed') : 'Not installed',
  );
  const actions = element('div', 'modn-tile__actions');
  const askHost = element('div', 'modn-tile__ask');
  body.append(state, actions, askHost);

  const settings = {
    askHost,
    onSettled: onChanged,
    newestOperation: hooks.newestOperation,
    runUndo: hooks.runUndo,
  };

  if (installed) {
    actions.append(
      createOperationButton({
        ...settings,
        label: 'Uninstall',
        action: 'Uninstall',
        ariaLabel: `Uninstall ${model.name}`,
        run: ({ runId }) =>
          uninstall({
            gameId: ctx.gameId,
            path: ctx.path,
            packageId: installed.id,
            runId,
          }),
      }).element,
    );
    if (model.hasUpdate) {
      actions.append(
        createOperationButton({
          ...settings,
          label: 'Update',
          action: 'Update',
          ariaLabel: `Update ${model.name}`,
          run: ({ runId }) =>
            update({
              gameId: ctx.gameId,
              path: ctx.path,
              packageId: installed.id,
              runId,
            }),
        }).element,
      );
    }
  } else {
    actions.append(
      createOperationButton({
        ...settings,
        label: 'Install',
        action: 'Install',
        ariaLabel: `Install ${model.name}`,
        run: ({ runId, approvals }) =>
          installMod({
            gameId: ctx.gameId,
            path: ctx.path,
            modId: model.modId,
            allowNativeInjector: approvals.native,
            allowExecutable: approvals.executable,
            runId,
          }),
      }).element,
    );
  }

  tile.append(art, body);
  return tile;
}
