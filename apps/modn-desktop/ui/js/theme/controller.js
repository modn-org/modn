/**
 * Loads, selects and imports the theme of the active game.
 *
 * The controller never spawns anything itself: the API, the apply function and
 * the toast are injectable. When the CLI has no theme commands the controller
 * leaves the default theme in place and reports `unavailable` without a toast,
 * so a missing CLI cannot flood the screen.
 */
import * as api from '../api.js';
import { messageFor } from '../messages.js';
import { showToast } from '../toast.js';
import { applyTheme, browserEnvironment } from './loader.js';
/** The game whose resolve answer the screen still waits for. */
let latestGame = null;
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
 * Resolves and applies the theme of one game.
 *
 * A stale answer is dropped: the game id is compared after the promise
 * settles, so navigating to another game cannot repaint the newer screen.
 * @param {string} game
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export async function loadThemeForGame(game, deps = {}) {
  const client = deps.api || api;
  const apply = deps.applyTheme || applyResolved;
  const toast = deps.toast || showToast;
  latestGame = game;
  let resolved;
  try {
    resolved = await client.resolveTheme(game);
  } catch (error) {
    return { applied: false, unavailable: true, message: messageFor(error) };
  }
  if (game !== latestGame) {
    return { applied: false, stale: true };
  }
  const result = apply(resolved);
  const diagnostics = Array.isArray(resolved.diagnostics) ? resolved.diagnostics : [];
  let notice = null;
  if (diagnostics.length > 0) {
    notice = diagnosticLine(diagnostics[0]);
  } else if (resolved.fallback) {
    notice = fallbackLine(resolved.fallback);
  } else if (!result.applied && result.error) {
    notice = `Theme not applied: ${result.error.message || 'the stylesheet is invalid'}`;
  }
  if (notice) {
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
 * Selects a theme and its variant, then resolves and applies the selection.
 * @param {string} game
 * @param {string} theme
 * @param {string|null} variant
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export async function selectAndApply(game, theme, variant, deps = {}) {
  const client = deps.api || api;
  await client.selectTheme(game, theme, variant === '' ? undefined : variant);
  return loadThemeForGame(game, deps);
}
/**
 * Clears the selection of one game, then resolves and applies the default.
 * @param {string} game
 * @param {{api?: Object, applyTheme?: Function, toast?: Function}} [deps]
 * @returns {Promise<Object>}
 */
export async function resetAndApply(game, deps = {}) {
  const client = deps.api || api;
  await client.resetTheme(game);
  return loadThemeForGame(game, deps);
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
  latestGame = null;
  return apply({ theme: null });
}
