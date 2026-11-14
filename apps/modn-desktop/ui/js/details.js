import { catalog, diagnostics, installMod, listPackages, planInstall, uninstall, update } from './api.js';
import { messageFor } from './messages.js';
import { createOperationButton } from './op-button.js';
import { showToast } from './toast.js';
import { element } from './details/dom.js';
import { createHistorySection, undoHooks } from './details/history.js';
import {
  bucketOf,
  describeMod,
  modelOf,
  stateLabel,
  versionOf,
} from './details/model.js';
import { createPlanSection } from './details/plan.js';

/**
 * @param {{gameId: string, path: string}} ctx
 * @returns {string}
 */
function libraryHref(ctx) {
  const path = ctx.path ? `?path=${encodeURIComponent(ctx.path)}` : '';
  return `#/game/${encodeURIComponent(ctx.gameId)}${path}`;
}

/**
 * @param {unknown} value
 * @returns {string}
 */
function asText(value) {
  return typeof value === 'string' ? value : JSON.stringify(value ?? '');
}

/**
 * @returns {Promise<void>}
 */
function noRun() {
  return Promise.resolve();
}

/**
 * The one-click operation that fits the current state of the mod.
 * @param {{entry: Object|null, installed: Object|null, hasUpdate: boolean}} model
 * @param {{gameId: string, modId: string, path: string}} ctx
 * @returns {Object}
 */
function actionFor(model, ctx) {
  const name = model.entry && typeof model.entry.name === 'string' ? model.entry.name : ctx.modId;
  if (!ctx.path) {
    return {
      label: 'Install',
      action: 'Install',
      ariaLabel: `Install ${name}`,
      run: noRun,
      blocked: 'Choose an installation first.',
    };
  }
  if (!model.installed) {
    if (!model.entry) {
      return {
        label: 'Install',
        action: 'Install',
        ariaLabel: `Install ${name}`,
        run: noRun,
        blocked: messageFor('MOD_NOT_FOUND'),
      };
    }
    return {
      label: 'Install',
      action: 'Install',
      ariaLabel: `Install ${name}`,
      run: ({ runId, approvals }) =>
        installMod({
          gameId: ctx.gameId,
          path: ctx.path,
          modId: ctx.modId,
          allowNativeInjector: approvals.native,
          allowExecutables: approvals.executables,
          runId,
        }),
    };
  }
  const installed = model.installed;
  if (model.hasUpdate) {
    return {
      label: 'Update',
      action: 'Update',
      ariaLabel: `Update ${name}`,
      run: ({ runId, approvals }) =>
        update({
          gameId: ctx.gameId,
          path: ctx.path,
          packageId: installed.id,
          allowNativeInjector: approvals.native,
          allowExecutables: approvals.executables,
          runId,
        }),
    };
  }
  return {
    label: 'Uninstall',
    action: 'Uninstall',
    ariaLabel: `Uninstall ${name}`,
    run: ({ runId }) =>
      uninstall({
        gameId: ctx.gameId,
        path: ctx.path,
        packageId: installed.id,
        runId,
      }),
  };
}

/**
 * Mod details view: one mod, its one-click operation, a collapsed file plan,
 * its operation history with Undo, and Copy diagnostics.
 * @param {HTMLElement} root
 * @param {{gameId: string, modId: string, path: string}} ctx
 */
export async function render(root, ctx) {
  const view = element('div', 'modn-view modn-details');
  view.dataset.testid = 'mod-details';
  view.dataset.gameId = ctx.gameId;
  view.dataset.modId = ctx.modId;

  const back = element('a', 'modn-link modn-details__back', '← Library');
  back.href = libraryHref(ctx);
  back.dataset.testid = 'details-back';
  back.setAttribute('aria-label', `Back to the ${ctx.gameId} library`);

  const header = element('header', 'modn-view__header modn-details__header');
  const titles = element('div', 'modn-row__text modn-details__titles');
  const title = element('h1', 'modn-details__title', ctx.modId);
  const meta = element('p', 'modn-muted modn-details__meta');
  meta.dataset.testid = 'details-meta';
  const description = element('p', 'modn-details__description', 'Loading mod…');
  description.dataset.testid = 'details-description';
  const state = element('p', 'modn-details__state');
  state.dataset.testid = 'details-state';
  titles.append(title, meta, description, state);
  const actions = element('div', 'modn-actions modn-details__actions');
  const askHost = element('div', 'modn-details__ask');
  header.append(titles, actions);

  const status = element('p', 'modn-status');
  status.dataset.testid = 'details-status';
  status.setAttribute('role', 'status');

  const plan = createPlanSection(ctx);
  const historySection = createHistorySection(ctx, { reload });
  const footer = element('footer', 'modn-details__footer');
  const copy = element('button', 'modn-button modn-button--secondary', 'Copy diagnostics');
  copy.type = 'button';
  copy.dataset.testid = 'copy-diagnostics';
  copy.setAttribute('aria-label', 'Copy diagnostics');
  copy.addEventListener('click', () => {
    void copyDiagnostics(copy);
  });
  footer.append(copy);

  view.append(back, header, status, askHost, plan, historySection.element, footer);
  root.append(view);

  /** @type {ReturnType<typeof createOperationButton>|null} */
  let operation = null;

  /**
   * @param {HTMLButtonElement} button
   */
  async function copyDiagnostics(button) {
    button.disabled = true;
    try {
      const report = await diagnostics();
      await navigator.clipboard.writeText(asText(report));
      showToast('Diagnostics copied.');
    } catch (error) {
      showToast(messageFor(error));
    } finally {
      button.disabled = false;
    }
  }

  /** @param {string} message */
  function showError(message) {
    status.textContent = message;
    status.classList.add('modn-status--error');
  }

  async function reload() {
    root.replaceChildren();
    await render(root, ctx);
  }

  /**
   * @param {{entry: Object|null, installed: Object|null, hasUpdate: boolean}} model
   */
  function renderAction(model) {
    if (operation) {
      operation.destroy();
      operation = null;
    }
    const choice = actionFor(model, ctx);
    operation = createOperationButton({
      ...undoHooks(ctx.gameId),
      askHost,
      label: choice.label,
      action: choice.action,
      ariaLabel: choice.ariaLabel,
      plan: () =>
        planInstall({
          gameId: ctx.gameId,
          path: ctx.path,
          modId: ctx.modId,
        }),
      run: choice.run,
      onSettled: reload,
      onError: showError,
    });
    if (choice.blocked) {
      operation.setBlocked(choice.blocked);
    }
    actions.replaceChildren(operation.element);
  }

  async function apply() {
    status.classList.remove('modn-status--error');
    status.textContent = 'Loading mod…';
    let model = { entry: null, installed: null, hasUpdate: false };
    if (!ctx.path) {
      meta.textContent = ctx.gameId;
      description.textContent = 'Choose an installation first.';
      state.textContent = 'No installation selected';
      renderAction(model);
      status.textContent = '';
    } else {
      try {
        const [entries, packages] = await Promise.all([
          catalog({ gameId: ctx.gameId, path: ctx.path, query: ctx.modId }),
          listPackages({ gameId: ctx.gameId, path: ctx.path }),
        ]);
        model = modelOf(ctx.modId, entries, packages);
        title.textContent =
          model.entry && typeof model.entry.name === 'string' ? model.entry.name : ctx.modId;
        meta.textContent = [versionOf(model), bucketOf(model)].filter(Boolean).join(' · ');
        description.textContent = describeMod(model.entry);
        state.textContent = stateLabel(model);
        renderAction(model);
        status.textContent = '';
      } catch (error) {
        const message = messageFor(error);
        showError(message);
        description.textContent = 'Mod details are unavailable.';
        state.textContent = '';
        renderAction(model);
        if (operation) {
          operation.setBlocked(message);
        }
      }
    }
    await historySection.load(model);
  }

  await apply();
}
