import { installMod, planInstall, undo, uninstall, update } from '../api.js';
import { createOperationButton } from '../op-button.js';
import { element, detailsHref, initials } from './dom.js';

/**
 * Shared Undo plumbing: the undo command every operation button uses.
 * @param {string} gameId
 * @returns {{runUndo: Function}}
 */
export function undoHooks(gameId) {
  return {
    runUndo: ({ runId, operationId, approvals }) =>
      undo({
        operationId,
        runId,
        allowNativeInjector: approvals.native,
        allowExecutables: approvals.executables,
      }),
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
 * Returns the identity of one action on one mod. The installed package owns the
 * mod once Modn installs it, so a running button is found under either the
 * catalog entry or the package row that replaces it.
 * @param {import('./data.js').LibraryModel} model
 * @returns {string}
 */
function operationSubject(model) {
  const installed = model.installedPackage;
  return installed ? `package-${installed.id}` : `${model.bucket}:${model.modId}`;
}

/**
 * @param {import('./data.js').LibraryModel} model
 * @param {{gameId: string, path: string}} ctx
 * @param {{
 *   onChanged: () => void|Promise<void>,
 *   operations: Map<string, {button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>,
 *   undoHost?: HTMLElement|null,
 * }} options
 * @returns {HTMLLIElement}
 */
export function createTile(model, ctx, { onChanged, operations, undoHost = null }) {
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

  const undoToast = undoHost
    ? createUndoLauncher({ host: undoHost, ...hooks, onSettled: onChanged })
    : null;
  const settings = {
    onSettled: onChanged,
    runUndo: hooks.runUndo,
    launchUndo: undoToast,
  };
  /** @type {Array<{button: ReturnType<typeof createOperationButton>, askHost: HTMLElement}>} */
  const buttons = [];
  const subject = operationSubject(model);

  if (installed) {
    buttons.push(
      operationEntry(operations, `uninstall:${subject}`, (askHost) =>
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
        operationEntry(operations, `update:${subject}`, (askHost) =>
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
      operationEntry(operations, `install:${subject}`, (askHost) =>
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

/**
 * Returns the Undo action a toast offers. Each action owns a fresh operation
 * button in a stable host, so a grid refresh may replace the button that
 * showed the toast without making its Undo a no-op. The fresh button offers
 * the same launcher again, so every later Undo starts from a live button.
 * The button keeps the operation ID across retries, and the temporary row
 * leaves the host as soon as the attempt settles, except while an approval
 * ask waits for the user.
 * @param {{
 *   host: HTMLElement,
 *   runUndo: (args: Object) => Promise<unknown>,
 *   onSettled: () => void|Promise<void>,
 * }} options
 * @returns {(operationId: string) => void}
 */
export function createUndoLauncher({ host, runUndo, onSettled }) {
  const launch = (operationId) => {
    if (!operationId) {
      return;
    }
    const row = element('div');
    const askHost = element('div', 'modn-tile__ask');
    const runner = createOperationButton({
      label: 'Undo',
      action: 'Undo',
      run: (args) => runUndo({ ...args, operationId }),
      testId: 'toast-undo-button',
      ariaLabel: 'Undo the last operation',
      askHost,
      runUndo,
      launchUndo: launch,
      onSettled: async () => {
        try {
          await onSettled();
        } finally {
          row.remove();
        }
      },
      onError: () => row.remove(),
      onBlocked: () => row.remove(),
      onDeclined: () => row.remove(),
    });
    row.append(runner.element, askHost);
    host.append(row);
    void runner.start({
      label: 'Undo',
      action: 'Undo',
      operationId,
      params: {},
      approvals: { native: false, executables: [] },
    });
  };
  return launch;
}
