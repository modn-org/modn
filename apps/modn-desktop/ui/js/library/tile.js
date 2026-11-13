import { history, installMod, planInstall, undo, uninstall, update } from '../api.js';
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
 * Returns the operation entry for one action of one tile. An entry whose run
 * is still going is moved into the new tile instead of replaced, so its
 * progress and Cancel survive a grid refresh. Idle entries are replaced,
 * because the tile's available actions may have changed.
 * @param {Map<string, {button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>} operations
 * @param {string} key
 * @param {(askHost: HTMLElement) => ReturnType<typeof createOperationButton>} build
 * @returns {{button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}}
 */
function operationEntry(operations, key, build) {
  const current = operations.get(key);
  if (current) {
    const buttonState = current.button.button.dataset.state;
    if (buttonState === 'running' || buttonState === 'finishing') {
      return current;
    }
    current.button.destroy();
    operations.delete(key);
  }
  const askHost = element('div', 'modn-tile__ask');
  const entry = { button: build(askHost), askHost };
  operations.set(key, entry);
  return entry;
}

/**
 * @param {import('./data.js').LibraryModel} model
 * @param {{gameId: string, path: string}} ctx
 * @param {{
 *   onChanged: () => void|Promise<void>,
 *   operations: Map<string, {button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>,
 * }} options
 * @returns {HTMLLIElement}
 */
export function createTile(model, ctx, { onChanged, operations }) {
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
  body.append(state, actions);

  const settings = {
    onSettled: onChanged,
    newestOperation: hooks.newestOperation,
    runUndo: hooks.runUndo,
  };
  /** @type {Array<{button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>} */
  const buttons = [];

  if (installed) {
    buttons.push(
      operationEntry(operations, `uninstall:${model.key}`, (askHost) =>
        createOperationButton({
          ...settings,
          askHost,
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
        }),
      ),
    );
    if (model.hasUpdate) {
      buttons.push(
        operationEntry(operations, `update:${model.key}`, (askHost) =>
          createOperationButton({
            ...settings,
            askHost,
            label: 'Update',
            action: 'Update',
            ariaLabel: `Update ${model.name}`,
            plan: () =>
              planInstall({
                gameId: ctx.gameId,
                path: ctx.path,
                modId: model.modId,
              }),
            run: ({ runId, approvals }) =>
              update({
                gameId: ctx.gameId,
                path: ctx.path,
                packageId: installed.id,
                allowNativeInjector: approvals.native,
                allowExecutables: approvals.executables,
                runId,
              }),
          }),
        ),
      );
    }
  } else {
    buttons.push(
      operationEntry(operations, `install:${model.key}`, (askHost) =>
        createOperationButton({
          ...settings,
          askHost,
          label: 'Install',
          action: 'Install',
          ariaLabel: `Install ${model.name}`,
          plan: () =>
            planInstall({
              gameId: ctx.gameId,
              path: ctx.path,
              modId: model.modId,
            }),
          run: ({ runId, approvals }) =>
            installMod({
              gameId: ctx.gameId,
              path: ctx.path,
              modId: model.modId,
              allowNativeInjector: approvals.native,
              allowExecutables: approvals.executables,
              runId,
            }),
        }),
      ),
    );
  }
  for (const entry of buttons) {
    actions.append(entry.button.element);
    body.append(entry.askHost);
  }

  tile.append(art, body);
  return tile;
}
