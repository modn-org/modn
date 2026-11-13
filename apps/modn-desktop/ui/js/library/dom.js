/**
 * DOM helpers shared by the Library view.
 */

/**
 * @param {string} tag
 * @param {string} [className]
 * @param {string} [text]
 * @returns {HTMLElement}
 */
export function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

/**
 * Builds a mod details route for one installation.
 * @param {{gameId: string, path: string}} ctx
 * @param {string} modId
 * @returns {string}
 */
export function detailsHref(ctx, modId) {
  const path = ctx.path ? `?path=${encodeURIComponent(ctx.path)}` : '';
  return `#/game/${encodeURIComponent(ctx.gameId)}/mod/${encodeURIComponent(modId)}${path}`;
}

/**
 * Two-letter placeholder for a tile without cover art.
 * @param {string} name
 * @returns {string}
 */
export function initials(name) {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.slice(0, 2).map((word) => word[0].toUpperCase());
  return letters.join('') || '?';
}
