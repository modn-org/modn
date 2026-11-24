/**
 * Loads, selects and imports the theme of the active game.
 *
 * The controller never spawns anything itself: the API, the apply function and
 * the toast are injectable. When the CLI has no theme commands the controller
 * leaves the default theme in place and reports `unavailable` without a toast,
 * so a missing CLI cannot flood the screen.
 *
 * Every load takes a ticket from one module-level generation counter. Only the
 * newest ticket may apply sheets, so an answer for a game the user left cannot
 * repaint the current screen. A load error for the current ticket removes the
 * sheets of the previous game.
 */
import * as api from '../api.js';
import { messageFor } from '../messages.js';
import { showToast } from '../toast.js';
import { applyTheme, browserEnvironment } from './loader.js';
/** The current load ticket; a newer request makes older answers stale. */
let generation = 0;
/** The game whose resolve answer the screen still waits for. */
let activeGame = null;
/** The most notices remembered for one session. */
const MAX_NOTICES = 50;
/** Notice keys already shown in this session, oldest first. */
const shownNotices = new Set();
/**
 * Applies one resolved answer. The sheet environment is created per call
 * because the webview owns the document.
 * @param {Object} resolved
 * @returns {Object}
 */
function applyResolved(resolved) {
  return applyTheme(resolved, browserEnvironment());
}
/**
 * One toast line for one diagnostic. The rule id and the file come first so a
 * person can find the rule in the theme authoring checklist.
 * @param {{rule?: string, path?: string|null, message?: string}} diagnostic
 * @returns {string}
 */
export function diagnosticLine(diagnostic) {
  const rule = diagnostic.rule || 'theme-error';
  const path = diagnostic.path || 'the theme';
  return `Theme not applied: ${rule} in ${path}: ${diagnostic.message || ''}`.trim();
}
/**
 * One toast line for a fallback without a diagnostic.
 * @param {{reason: string, message: string}} fallback
 * @returns {string}
 */
function fallbackLine(fallback) {
  return `Theme not applied: ${fallback.reason}: ${fallback.message}`;
}
/**
 * Reports whether one diagnostic explains the active theme rather than another
 * theme of the same game. The panel already lists catalog problems, so a
 * diagnostic about a different theme must not claim the selected theme failed.
 * @param {{theme?: string|null}} diagnostic
 * @param {string} game
 * @param {Object} resolved
 * @returns {boolean}
 */
function concernsActiveTheme(diagnostic, game, resolved) {
  if (!resolved.theme || resolved.fallback) {
    return true;
  }
  return diagnostic.theme === `${game}/${resolved.theme.id}`;
}
/**
 * Remembers one notice key for the session and reports whether it is new.
 * @param {string} key
 * @returns {boolean}
 */
function rememberNotice(key) {
  if (shownNotices.has(key)) {
    return false;
  }
  if (shownNotices.size >= MAX_NOTICES) {
    const oldest = shownNotices.values().next().value;
    shownNotices.delete(oldest);
  }
  shownNotices.add(key);
  return true;
}
/**
 * Builds the notice key of one resolved answer and its toast text.
 * @param {string} game
 * @param {Object} resolved
 * @param {string} text
 * @returns {string}
 */
function noticeKey(game, resolved, text) {
  const theme = resolved.theme || {};
  return `${game}|${theme.id || ''}|${theme.variant || ''}|${text}`;
}
/** Forgets every notice shown so far. Tests use it between scenarios. */
export function resetNoticeMemory() {
  shownNotices.clear();
}
/**
 * Resolves and applies the theme of one game.
 *
 * A stale answer is dropped: the ticket is compared after the promise settles,
 * so navigating to another game cannot repaint the newer screen. A failed
 * current request removes the previous game's sheets before it reports
 * `unavailable`.
 * @param {string} game
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export async function loadThemeForGame(game, deps = {}) {
  const client = deps.api || api;
  const apply = deps.applyTheme || applyResolved;
  const toast = deps.toast || showToast;
  const ticket = ++generation;
  activeGame = game;
  let resolved;
  try {
    resolved = await client.resolveTheme(game);
  } catch (error) {
    if (ticket !== generation) {
      return { applied: false, stale: true };
    }
    apply({ theme: null });
    return { applied: false, unavailable: true, message: messageFor(error) };
  }
  if (ticket !== generation || (resolved && resolved.stale === true)) {
    return { applied: false, stale: true };
  }
  const result = apply(resolved);
  const diagnostics = Array.isArray(resolved.diagnostics) ? resolved.diagnostics : [];
  const active = diagnostics.find((diagnostic) => concernsActiveTheme(diagnostic, game, resolved));
  let notice = null;
  if (active) {
    notice = diagnosticLine(active);
  } else if (resolved.fallback) {
    notice = fallbackLine(resolved.fallback);
  } else if (!result.applied && result.error) {
    notice = `Theme not applied: ${result.error.message || 'the stylesheet is invalid'}`;
  }
  if (notice && rememberNotice(noticeKey(game, resolved, notice))) {
    toast(notice);
  }
  return {
    ...result,
    unavailable: false,
    disabled: resolved.disabled || null,
    diagnostics,
  };
}
/**
 * Selections running or queued. A new selection waits for the older ones so
 * that two quick choices store and apply in click order.
 */
let selectionDepth = 0;
/** Settles when the newest queued selection ends. */
let selectionTail = Promise.resolve();
/**
 * Runs one selection. The first one starts at once; a later one waits for the
 * older selection.
 * @template T
 * @param {() => Promise<T>} run
 * @returns {Promise<T>}
 */
function queueSelection(run) {
  const next = selectionDepth === 0 ? run() : selectionTail.then(run, run);
  selectionDepth += 1;
  selectionTail = next.then(
    () => {
      selectionDepth -= 1;
    },
    () => {
      selectionDepth -= 1;
    },
  );
  return next;
}
/**
 * Stores one selection and applies its theme unless the game already changed.
 * @param {string} game
 * @param {() => Promise<Object>} store
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} deps
 * @returns {Promise<Object>}
 */
async function storeAndApply(game, store, deps) {
  if (activeGame !== game) {
    return { applied: false, stale: true };
  }
  const start = generation;
  await store();
  if (generation !== start || activeGame !== game) {
    return { applied: false, stale: true };
  }
  return loadThemeForGame(game, deps);
}
/**
 * Selects a theme and its variant, then resolves and applies the selection.
 *
 * The generation and game are checked after the selection settles, so a
 * selection that finishes after the user navigated away applies nothing.
 * @param {string} game
 * @param {string} theme
 * @param {string|null} variant
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export function selectAndApply(game, theme, variant, deps = {}) {
  const client = deps.api || api;
  return queueSelection(() =>
    storeAndApply(
      game,
      () => client.selectTheme(game, theme, variant === '' ? undefined : variant),
      deps,
    ),
  );
}
/**
 * Stores the explicit no-theme choice, then resolves and applies the default.
 *
 * The generation and game are checked after the selection settles, so a
 * selection that finishes after the user navigated away applies nothing.
 * @param {string} game
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export function selectNoneAndApply(game, deps = {}) {
  const client = deps.api || api;
  return queueSelection(() => storeAndApply(game, () => client.selectTheme(game, null), deps));
}
/**
 * Clears the selection of one game, then resolves and applies the default.
 * @param {string} game
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export function resetAndApply(game, deps = {}) {
  const client = deps.api || api;
  return queueSelection(() => storeAndApply(game, () => client.resetTheme(game), deps));
}
/**
 * Registers an imported theme and returns the answer unchanged.
 *
 * A refusal that carries diagnostics is returned rather than thrown, so the
 * panel can list each rule. Other failures still reject.
 * @param {string} game
 * @param {{api?: Object}} [deps]
 * @returns {Promise<Object>}
 */
export async function importAndRefresh(game, deps = {}) {
  const client = deps.api || api;
  try {
    return await client.importTheme();
  } catch (error) {
    if (error && Array.isArray(error.diagnostics)) {
      return error;
    }
    throw error;
  }
}
/**
 * Removes the theme sheets and drops any pending answer, so the shell returns
 * to the default theme when no game is active.
 * @param {{applyTheme?: Function}} [deps]
 * @returns {Object}
 */
export function clearTheme(deps = {}) {
  const apply = deps.applyTheme || applyResolved;
  generation += 1;
  activeGame = null;
  return apply({ theme: null });
}
