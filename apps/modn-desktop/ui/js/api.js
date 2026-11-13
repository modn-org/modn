/**
 * The only module that touches the Tauri global. Every wrapper returns the
 * backend JSON unchanged; failures reject with `{code, message}`.
 * @typedef {Object} Game
 * @property {string} game_id
 * @property {string} path
 * @property {string} [source]
 *
 * @typedef {Object} OwnedFile
 * @property {string} root
 * @property {string} destination
 * @property {string} sha256
 * @property {number} bytes
 *
 * @typedef {Object} ManagedPackage
 * @property {string} id
 * @property {boolean} pinned
 * @property {string} game_installation
 * @property {string} archive_sha256
 * @property {OwnedFile[]} files
 * @property {string[]} directories
 * @property {{bucket: string, game_id: string, mod_id: string}|null} catalog
 * @property {string} [mod_id]
 * @property {{kind: string, filename?: string}} source
 * @property {string} [archive_blake3]
 *
 * @typedef {Object} BucketPackage
 * @property {string} bucket
 * @property {number} schema_version
 * @property {string} id
 * @property {string} game
 * @property {string} name
 * @property {string} version
 * @property {string} [archive_format]
 * @property {string} [sha256]
 * @property {Object} source
 * @property {string[]} dependencies
 * @property {string} [homepage]
 * @property {boolean} native_injector
 * @property {string[]} [executables]
 *
 * @typedef {Object} PlanOperation
 * @property {string} source
 * @property {string} destination
 * @property {number} bytes
 * @property {string} [kind]
 * @property {string} [root]
 *
 * @typedef {Object} PlanBlocker
 * @property {string} code
 * @property {string} message
 *
 * @typedef {Object} InstallPlan
 * @property {string} archive_sha256
 * @property {string} game_installation
 * @property {'reviewable'|'blocked'} status
 * @property {boolean} metadata_only
 * @property {PlanOperation[]} operations
 * @property {PlanBlocker[]} blockers
 * @property {string[]} skipped_generated_files
 *
 * @typedef {Object} ProgressEvent
 * @property {'resolve'|'download'|'extract'|'stage'|'commit'|'result'} phase
 * @property {string} [package]
 * @property {number} [bytes]
 * @property {number|null} [total]
 * @property {'succeeded'|'failed'|'cancelled'} [outcome]
 * @property {string} [error]
 */

function invoke(command, args) {
  return window.__TAURI__.core.invoke(command, args);
}

/** Creates the per-operation id used by progress events. */
export function newRunId() {
  if (window.crypto && typeof window.crypto.randomUUID === 'function') {
    return window.crypto.randomUUID();
  }
  return `run-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

/** @returns {Promise<Game[]>} `modn game list` installations. */
export function listGames() {
  return invoke('list_games', {});
}

/** @param {{gameId: string}} args @returns {Promise<Game[]>} installations of one game. */
export function listInstallations({ gameId }) {
  return invoke('list_installations', { gameId });
}

/**
 * @param {{gameId: string, path: string}} args
 * @returns {Promise<ManagedPackage[]>} manager-owned packages for one installation.
 */
export function listPackages({ gameId, path }) {
  return invoke('list_packages', { gameId, path });
}

/**
 * @param {{gameId: string, path: string, query?: string}} args
 * @returns {Promise<BucketPackage[]>} bucket catalog entries for the game.
 */
export function catalog({ gameId, path, query }) {
  return invoke('catalog', { gameId, path, query });
}

/** @returns {Promise<string|null>} chosen `.zip` or `.7z` path; null on cancel. */
export function pickArchive() {
  return invoke('pick_archive', {});
}

/**
 * One-click install of a local archive. Runs the dry run and rechecks internally.
 * @param {{gameId: string, path: string, archive: string,
 *   allowNativeInjector: boolean, allowExecutable: boolean, runId: string}} args
 * @returns {Promise<ManagedPackage>}
 */
export function installArchive({
  gameId,
  path,
  archive,
  allowNativeInjector,
  allowExecutable,
  runId,
}) {
  return invoke('install_archive', {
    gameId,
    path,
    archive,
    allowNativeInjector,
    allowExecutable,
    runId,
  });
}

/**
 * One-click uninstall of an owned package.
 * @param {{gameId: string, path: string, packageId: string, runId: string}} args
 * @returns {Promise<{uninstalled: string, packages: string[]}>}
 */
export function uninstall({ gameId, path, packageId, runId }) {
  return invoke('uninstall', { gameId, path, packageId, runId });
}

/**
 * Catalog install by mod id. Implemented by a later task.
 * @param {{gameId: string, path: string, modId: string,
 *   allowNativeInjector: boolean, allowExecutable: boolean, runId: string}} args
 * @returns {Promise<{package: string, installed: ManagedPackage}>}
 */
export function installMod(args) {
  return invoke('install_mod', args);
}

/**
 * Read-only install plan for the details screen. Implemented by a later task.
 * @param {{gameId: string, path: string, modId: string}} args
 * @returns {Promise<InstallPlan|Object>} standalone plan, or catalog plan with
 *   `plan`, `approval`, `dependencies` and `blockers`.
 */
export function planInstall({ gameId, path, modId }) {
  return invoke('plan_install', { gameId, path, modId });
}

/**
 * Update one owned package. Implemented by a later task.
 * @param {{gameId: string, path: string, packageId: string, runId: string}} args
 * @returns {Promise<Object>} update report with `plan`, `files` and `blockers`.
 */
export function update({ gameId, path, packageId, runId }) {
  return invoke('update', { gameId, path, packageId, runId });
}

/**
 * Operation history, newest first. Implemented by a later task.
 * @param {{gameId: string}} args
 * @returns {Promise<Array<{id: string, kind: string, game_id: string,
 *   installation: string, recorded_at: string, packages: Object[], undoes?: string}>>}
 */
export function history({ gameId }) {
  return invoke('history', { gameId });
}

/**
 * Undo one operation. Implemented by a later task.
 * @param {{operationId: string, runId: string}} args
 * @returns {Promise<{kind: string, installed: Object[], removed: string[], undo: string}>}
 */
export function undo({ operationId, runId }) {
  return invoke('undo', { operationId, runId });
}

/**
 * Detect installed games without writing state. Implemented by a later task.
 * @returns {Promise<{installations: Object[], refused: Object[], skipped: Object[]}>}
 */
export function detectGames() {
  return invoke('detect_games', {});
}

/**
 * Register a detected installation. Implemented by a later task.
 * @param {{gameId: string, path: string}} args @returns {Promise<Game>}
 */
export function activateGame({ gameId, path }) {
  return invoke('activate_game', { gameId, path });
}

/**
 * Pick a game folder with a native picker, then register it. Later task.
 * @param {{gameId: string}} args @returns {Promise<Game|null>}
 */
export function addGame({ gameId }) {
  return invoke('add_game', { gameId });
}

/**
 * Unregister an installation after a native confirmation. Later task.
 * @param {{gameId: string, path: string}} args @returns {Promise<Game>}
 */
export function removeGame({ gameId, path }) {
  return invoke('remove_game', { gameId, path });
}

/**
 * Launch a registered installation. Later task.
 * @param {{gameId: string, path: string}} args
 * @returns {Promise<{game_id: string, installation: string, target: Object, warnings: string[]}>}
 */
export function play({ gameId, path }) {
  return invoke('play', { gameId, path });
}

/**
 * Support text for Copy diagnostics. Later task.
 * @returns {Promise<string>}
 */
export function diagnostics() {
  return invoke('diagnostics', {});
}

/**
 * Request cancellation of a run. Later task.
 * @param {{runId: string}} args @returns {Promise<void>}
 */
export function cancelRun({ runId }) {
  return invoke('cancel_run', { runId });
}

/**
 * Subscribes to progress for one run. Later tasks emit the events.
 * @param {string} runId
 * @param {(event: ProgressEvent) => void} callback
 * @returns {Promise<() => void>} unlisten function
 */
export function onProgress(runId, callback) {
  return window.__TAURI__.event.listen('modn://progress', (event) => {
    const payload = event.payload;
    if (payload && payload.runId === runId) {
      callback(payload.event);
    }
  });
}
