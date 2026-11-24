/**
 * The Theme control: one button in the app shell and a panel that lists the
 * themes of the active game, selects a theme and a variant, resets to the
 * default theme and imports a local theme.
 *
 * The panel is a plain section, not a modal dialog. It builds every node with
 * `element` and `textContent`, so theme text never becomes markup. Every class
 * starts with `modn-theme-`, the prefix reserved for this control.
 */
import * as api from '../api.js';
import { messageFor } from '../messages.js';
import { element } from '../details/dom.js';
import {
  importAndRefresh,
  loadThemeForGame,
  selectAndApply,
  selectNoneAndApply,
} from './controller.js';
/** The panel id that the trigger names with `aria-controls`. */
const PANEL_ID = 'modn-theme-panel';
/**
 * Creates the control and appends it to `host`.
 * @param {HTMLElement} host
 * @param {{api?: Object, loadThemeForGame?: Function,
 *   selectAndApply?: Function, selectNoneAndApply?: Function,
 *   importAndRefresh?: Function}} [deps]
 * @returns {{element: HTMLElement, setGame: Function, setTheme: Function,
 *   isOpen: Function}}
 */
export function createThemePanel(host, deps = {}) {
  const client = deps.api || api;
  const load = deps.loadThemeForGame || loadThemeForGame;
  const select = deps.selectAndApply || selectAndApply;
  const selectNone = deps.selectNoneAndApply || selectNoneAndApply;
  const importTheme = deps.importAndRefresh || importAndRefresh;
  const root = element('div', 'modn-theme-control');
  const trigger = element('button', 'modn-button modn-button--quiet modn-theme-button', 'Theme');
  trigger.type = 'button';
  trigger.dataset.testid = 'theme-button';
  trigger.setAttribute('aria-expanded', 'false');
  trigger.setAttribute('aria-controls', PANEL_ID);
  const panel = element('section', 'modn-theme-panel');
  panel.id = PANEL_ID;
  panel.hidden = true;
  panel.setAttribute('aria-label', 'Theme');
  root.append(trigger, panel);
  if (host) {
    host.append(root);
  }
  let game = null;
  let generation = 0;
  let open = false;
  let themes = [];
  let listDiagnostics = [];
  let importDiagnostics = [];
  let chosen = null;
  let variant = '';
  let current = { name: null, sourceLabel: null };
  let unavailable = null;
  let disabled = null;
  let message = null;
  /** Adds one text line to `parent`. */
  function line(parent, className, text) {
    const node = element('p', className, text);
    parent.append(node);
    return node;
  }
  /** One diagnostic as `<rule>: <message>`; a rule id is optional. */
  function diagnosticItem(diagnostic) {
    const rule = diagnostic.rule ? `${diagnostic.rule}: ` : '';
    return element('li', 'modn-theme-diagnostic', `${rule}${diagnostic.message || ''}`);
  }
  /** Applies a load result to the visible state. */
  function setTheme(result) {
    if (!result || result.stale) {
      return;
    }
    if (result.unavailable) {
      unavailable = result.message || messageFor(null);
    } else {
      unavailable = null;
    }
    disabled = result.disabled || null;
    if (result.applied) {
      current = {
        name: result.name || null,
        sourceLabel: (result.source && result.source.label) || null,
      };
      variant = result.variant || '';
    } else if (!result.unavailable && !result.disabled) {
      current = { name: null, sourceLabel: null };
    }
    render();
  }
  /** Fetches the list for the active game, ignoring an answer for an old one. */
  async function refresh() {
    if (!game || disabled) {
      render();
      return;
    }
    const expected = generation;
    try {
      const answer = await client.listThemes(game);
      if (expected !== generation) {
        return;
      }
      themes = Array.isArray(answer.themes) ? answer.themes : [];
      listDiagnostics = Array.isArray(answer.diagnostics) ? answer.diagnostics : [];
      unavailable = null;
      if (!chosen || !themes.some((theme) => theme.id === chosen)) {
        const selected = themes.find((theme) => theme.selected);
        chosen = selected ? selected.id : null;
      }
    } catch (error) {
      if (expected !== generation) {
        return;
      }
      themes = [];
      unavailable = messageFor(error);
    }
    render();
  }
  /** Builds the list of themes with a mark on the selected one. */
  function renderList() {
    const list = element('ul', 'modn-theme-list');
    list.dataset.testid = 'theme-list';
    for (const theme of themes) {
      const item = element('li', 'modn-theme-item');
      const picked = theme.id === chosen;
      const pick = element(
        'button',
        `modn-theme-pick${picked ? ' modn-theme-pick--chosen' : ''}`,
      );
      pick.type = 'button';
      pick.setAttribute('aria-pressed', String(picked));
      pick.append(element('span', 'modn-theme-name', theme.name || theme.id));
      const source = theme.source && theme.source.label ? theme.source.label : 'unknown source';
      pick.append(element('span', 'modn-theme-source', source));
      if (theme.selected) {
        pick.append(element('span', 'modn-theme-mark', 'Selected'));
      }
      pick.addEventListener('click', () => {
        chosen = theme.id;
        if (!picked) {
          variant = '';
        }
        importDiagnostics = [];
        render();
      });
      item.append(pick);
      list.append(item);
    }
    panel.append(list);
  }
  /** Builds the variant selector of the chosen theme. */
  function renderVariants() {
    const theme = themes.find((entry) => entry.id === chosen);
    if (!theme) {
      return;
    }
    const field = element('label', 'modn-theme-variant');
    field.append(element('span', 'modn-theme-source', 'Variant'));
    const choices = Array.isArray(theme.variants) ? theme.variants : [];
    const selectElement = element('select', 'modn-select modn-theme-select');
    selectElement.dataset.testid = 'theme-variant';
    const none = document.createElement('option');
    none.value = '';
    none.textContent = 'No variant';
    selectElement.append(none);
    for (const name of choices) {
      const option = document.createElement('option');
      option.value = name;
      option.textContent = name;
      option.selected = name === variant;
      selectElement.append(option);
    }
    selectElement.value = choices.includes(variant) ? variant : '';
    selectElement.addEventListener('change', () => {
      variant = selectElement.value;
    });
    field.append(selectElement);
    panel.append(field);
  }
  /** Adds the three explicit actions. */
  function renderActions() {
    const actions = element('div', 'modn-theme-actions');
    const use = element('button', 'modn-button modn-theme-action', 'Use theme');
    use.type = 'button';
    use.dataset.testid = 'theme-use';
    use.disabled = !chosen || Boolean(disabled);
    use.addEventListener('click', async () => {
      if (!game || !chosen) {
        return;
      }
      const expected = generation;
      const target = game;
      const theme = chosen;
      const selected = variant || null;
      try {
        const result = await select(target, theme, selected);
        if (expected !== generation) {
          return;
        }
        setTheme(result);
        await refresh();
      } catch (error) {
        if (expected !== generation) {
          return;
        }
        message = messageFor(error);
        render();
      }
    });
    const defaultButton = element('button', 'modn-button modn-button--quiet modn-theme-action', 'Use default theme');
    defaultButton.type = 'button';
    defaultButton.dataset.testid = 'theme-reset';
    defaultButton.disabled = Boolean(disabled);
    defaultButton.addEventListener('click', async () => {
      if (!game) {
        return;
      }
      const expected = generation;
      const target = game;
      try {
        const result = await selectNone(target);
        if (expected !== generation) {
          return;
        }
        chosen = null;
        variant = '';
        setTheme(result);
        await refresh();
      } catch (error) {
        if (expected !== generation) {
          return;
        }
        message = messageFor(error);
        render();
      }
    });
    const importButton = element('button', 'modn-button modn-button--quiet modn-theme-action', 'Import theme');
    importButton.type = 'button';
    importButton.dataset.testid = 'theme-import';
    importButton.disabled = Boolean(disabled);
    importButton.addEventListener('click', async () => {
      if (!game) {
        return;
      }
      const expected = generation;
      try {
        const answer = await importTheme(game);
        if (expected !== generation) {
          return;
        }
        if (!answer || answer.cancelled) {
          return;
        }
        importDiagnostics = Array.isArray(answer.diagnostics) ? answer.diagnostics : [];
        if (importDiagnostics.length === 0) {
          await refresh();
        } else {
          render();
        }
      } catch (error) {
        if (expected !== generation) {
          return;
        }
        message = error && Array.isArray(error.diagnostics) ? null : messageFor(error);
        importDiagnostics = error && Array.isArray(error.diagnostics) ? error.diagnostics : [];
        render();
      }
    });
    actions.append(use, defaultButton, importButton);
    panel.append(actions);
  }
  /** Rebuilds the panel from the state. */
  function render() {
    trigger.setAttribute('aria-expanded', String(open));
    panel.replaceChildren();
    if (unavailable) {
      line(panel, 'modn-theme-status', `Themes are unavailable: ${unavailable}`);
      return;
    }
    if (!game) {
      line(panel, 'modn-theme-status', 'Open a game to choose its theme.');
      return;
    }
    const currentBlock = element('div', 'modn-theme-current');
    currentBlock.append(element('p', 'modn-theme-name', current.name || 'Default theme'));
    if (current.name && current.sourceLabel) {
      currentBlock.append(element('p', 'modn-theme-source', current.sourceLabel));
    }
    panel.append(currentBlock);
    if (disabled) {
      line(panel, 'modn-theme-status', 'Themes are off for this session');
      renderActions();
      return;
    }
    if (message) {
      line(panel, 'modn-theme-status', message);
    }
    if (themes.length > 0) {
      renderList();
    } else {
      line(panel, 'modn-theme-status', 'No themes for this game.');
    }
    renderVariants();
    renderActions();
    const diagnostics = [...listDiagnostics, ...importDiagnostics];
    if (diagnostics.length > 0) {
      const list = element('ul', 'modn-theme-diagnostics');
      for (const diagnostic of diagnostics) {
        list.append(diagnosticItem(diagnostic));
      }
      panel.append(list);
    }
  }
  /** Opens or closes the panel. */
  function setOpen(next) {
    open = next;
    panel.hidden = !open;
    message = null;
    render();
    if (open) {
      void refresh();
    }
  }
  trigger.addEventListener('click', () => {
    setOpen(!open);
  });
  root.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape' || !open) {
      return;
    }
    event.preventDefault();
    setOpen(false);
    trigger.focus();
  });
  render();
  return {
    /** The control root, for tests and for the shell. */
    element: root,
    /** Points the control at the active game; `null` clears it. */
    setGame(next) {
      if (next === game) {
        return;
      }
      game = next || null;
      generation += 1;
      themes = [];
      listDiagnostics = [];
      importDiagnostics = [];
      chosen = null;
      variant = '';
      current = { name: null, sourceLabel: null };
      unavailable = null;
      disabled = null;
      message = null;
      render();
      if (open) {
        void refresh();
      }
    },
    /** Shows the result of a load for the active game. */
    setTheme,
    /** Reports whether the panel is open. */
    isOpen() {
      return open;
    },
  };
}
