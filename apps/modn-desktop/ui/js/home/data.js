/**
 * Loads and normalises what the Home view shows: registered installations and
 * detected candidates that are not registered yet.
 */

import { detectGames, listGames, listPackages } from '../api.js';

/** Game ids offered by Add when nothing is registered or detected. */
const FALLBACK_GAME_IDS = ['balatro', 'cyberpunk-2077'];

const GAME_LABELS = {
  balatro: 'Balatro',
  'cyberpunk-2077': 'Cyberpunk 2077',
};

const REGISTERED_STATUSES = new Set(['registered', 'already_registered']);

/**
 * Turns a game id into a short display name.
 * @param {string} gameId
 * @returns {string}
 */
export function gameLabel(gameId) {
  const id = String(gameId);
  if (Object.prototype.hasOwnProperty.call(GAME_LABELS, id)) {
    return GAME_LABELS[id];
  }
  const words = id.split(/[-_\s]+/).filter(Boolean);
  if (words.length === 0) {
    return id;
  }
  return words
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * Compares installation paths ignoring a trailing separator and letter case.
 * @param {string} path
 * @returns {string}
 */
function pathKey(path) {
  return String(path).replace(/[\\/]+$/, '').toLowerCase();
}

/**
 * @param {{gameId: string, path: string}} left
 * @param {{gameId: string, path: string}} right
 * @returns {boolean}
 */
function sameInstallation(left, right) {
  return left.gameId === right.gameId && pathKey(left.path) === pathKey(right.path);
}

/**
 * @param {unknown} games
 * @returns {Array<{gameId: string, path: string, state: string, store: string, canRemove: boolean}>}
 */
function registeredEntries(games) {
  const entries = [];
  const seen = new Set();
  for (const game of Array.isArray(games) ? games : []) {
    if (!game || typeof game.game_id !== 'string' || typeof game.path !== 'string') {
      continue;
    }
    const entry = {
      gameId: game.game_id,
      path: game.path,
      state: 'registered',
      store: typeof game.source === 'string' ? game.source : 'manual',
      canRemove: false,
    };
    const key = `${entry.gameId}\u0000${pathKey(entry.path)}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    entries.push(entry);
  }
  return entries;
}

/**
 * Keeps detected candidates that Modn does not manage yet.
 * @param {unknown} detection
 * @param {Array<{gameId: string, path: string}>} registered
 * @returns {Array<{gameId: string, path: string, state: string, store: string}>}
 */
function detectedEntries(detection, registered) {
  const installations =
    detection && Array.isArray(detection.installations) ? detection.installations : [];
  const entries = [];
  for (const item of installations) {
    if (!item || typeof item.game_id !== 'string' || typeof item.path !== 'string') {
      continue;
    }
    if (REGISTERED_STATUSES.has(item.status)) {
      continue;
    }
    const entry = {
      gameId: item.game_id,
      path: item.path,
      state: 'detected',
      store: typeof item.store === 'string' ? item.store : 'unknown',
    };
    if (registered.some((known) => sameInstallation(known, entry))) {
      continue;
    }
    entries.push(entry);
  }
  entries.sort((left, right) => gameLabel(left.gameId).localeCompare(gameLabel(right.gameId)));
  return entries;
}

/**
 * Marks installations that own no managed package, so Remove may show.
 * A failed listing keeps Remove hidden: emptiness is unconfirmed.
 * @param {Array<{gameId: string, path: string, canRemove: boolean}>} entries
 * @returns {Promise<void>}
 */
async function markRemovable(entries) {
  await Promise.all(
    entries.map(async (entry) => {
      try {
        const packages = await listPackages({ gameId: entry.gameId, path: entry.path });
        entry.canRemove = Array.isArray(packages) && packages.length === 0;
      } catch {
        entry.canRemove = false;
      }
    }),
  );
}

/**
 * Reads registered installations and detection candidates together.
 * A failed registration listing blocks the grid: the state is unknown.
 * @returns {Promise<{entries: Array<object>, listError: object|null,
 *   detectError: object|null}>}
 */
export async function loadHome() {
  const [gamesResult, detectionResult] = await Promise.allSettled([listGames(), detectGames()]);
  if (gamesResult.status === 'rejected') {
    return { entries: [], listError: gamesResult.reason, detectError: null };
  }
  const registered = registeredEntries(gamesResult.value);
  const detected =
    detectionResult.status === 'fulfilled'
      ? detectedEntries(detectionResult.value, registered)
      : [];
  await markRemovable(registered);
  return {
    entries: [...registered, ...detected],
    listError: null,
    detectError: detectionResult.status === 'rejected' ? detectionResult.reason : null,
  };
}

/**
 * Game ids the Add menu offers: every supported game and every game that
 * already has a card, without duplicates.
 * @param {Array<{gameId: string}>} entries
 * @returns {string[]}
 */
export function supportedGameIds(entries) {
  const ids = new Set(FALLBACK_GAME_IDS);
  for (const entry of Array.isArray(entries) ? entries : []) {
    if (entry && typeof entry.gameId === 'string' && entry.gameId) {
      ids.add(entry.gameId);
    }
  }
  return [...ids].sort();
}

/**
 * Names the supported games in one sentence.
 * @param {string[]} gameIds
 * @returns {string}
 */
export function supportedSentence(gameIds) {
  const labels = gameIds.map(gameLabel);
  if (labels.length === 1) {
    return labels[0];
  }
  return `${labels.slice(0, -1).join(', ')} and ${labels[labels.length - 1]}`;
}
