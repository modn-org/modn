/**
 * The only module that touches the Tauri global. Every wrapper returns the
 * backend JSON unchanged; failures reject with `{code, message, approval?}`.
 * The optional `approval` names the capabilities a refusal left uncovered, so
 * the ask flow can resend them as `allowNativeInjector` and `allowExecutables`.
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
 * @typedef {Object} Approval
 * @property {boolean} required
 * @property {{native_injector: boolean, executables: string[]}} uncovered
 *
 * @typedef {Object} ApprovalRequest
 * @property {boolean} nativeInjector
 * @property {string[]} executables
 *
 * @typedef {Object} CommandError
 * @property {string} code
 * @property {string} message
 * @property {ApprovalRequest} [approval]
 *
 * @typedef {Object} InstallPlan
 * @property {string} archive_sha256
 * @property {string} game_installation
 * @property {'reviewable'|'blocked'} status
 * @property {boolean} metadata_only
 * @property {PlanOperation[]} operations
 * @property {PlanBlocker[]} blockers
 * @property {string[]} skipped_generated_files
 * @property {Approval} [approval]
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
export async function listGames() {
  return invoke('list_games', {});
}

/** @param {{gameId: string}} args @returns {Promise<Game[]>} installations of one game. */
export async function listInstallations({ gameId }) {
  return invoke('list_installations', { gameId });
}

/**
 * @param {{gameId: string, path: string}} args
 * @returns {Promise<ManagedPackage[]>} manager-owned packages for one installation.
 */
export async function listPackages({ gameId, path }) {
  return invoke('list_packages', { gameId, path });
}

/**
 * @param {{gameId: string, path: string, query?: string}} args
 * @returns {Promise<BucketPackage[]>} bucket catalog entries for the game.
 */
export async function catalog({ gameId, path, query }) {
  return invoke('catalog', { gameId, path, query });
}

/** @returns {Promise<string|null>} chosen `.zip` or `.7z` path; null on cancel. */
export async function pickArchive() {
  return invoke('pick_archive', {});
}

/**
 * One-click install of a local archive. Runs the dry run and rechecks internally.
 * @param {{gameId: string, path: string, archive: string,
 *   allowNativeInjector: boolean, allowExecutables: string[], runId: string}} args
 * @returns {Promise<ManagedPackage>}
 */
export async function installArchive({
  gameId,
  path,
  archive,
  allowNativeInjector,
  allowExecutables = [],
  runId,
}) {
  return invoke('install_archive', {
    gameId,
    path,
    archive,
    allowNativeInjector,
    allowExecutables,
    runId,
  });
}

/**
 * One-click uninstall of an owned package.
 * @param {{gameId: string, path: string, packageId: string, runId: string}} args
 * @returns {Promise<{uninstalled: string, packages: string[]}>}
 */
export async function uninstall({ gameId, path, packageId, runId }) {
  return invoke('uninstall', { gameId, path, packageId, runId });
}

/**
 * Catalog install by mod id.
 * @param {{gameId: string, path: string, modId: string,
 *   allowNativeInjector: boolean, allowExecutables: string[], runId: string}} args
 * @returns {Promise<{package: string, installed: ManagedPackage}>}
 */
export async function installMod({
  gameId,
  path,
  modId,
  allowNativeInjector,
  allowExecutables = [],
  runId,
}) {
  return invoke('install_mod', {
    gameId,
    path,
    modId,
    allowNativeInjector,
    allowExecutables,
    runId,
  });
}

/**
 * Read-only install plan for the details screen. Implemented by a later task.
 * @param {{gameId: string, path: string, modId: string}} args
 * @returns {Promise<InstallPlan|Object>} standalone plan, or catalog plan with
 *   `plan`, `approval`, `dependencies` and `blockers`.
 */
export async function planInstall({ gameId, path, modId }) {
  return invoke('plan_install', { gameId, path, modId });
}

/**
 * Update one owned package.
 * @param {{gameId: string, path: string, packageId: string,
 *   allowNativeInjector: boolean, allowExecutables: string[], runId: string}} args
 * @returns {Promise<Object>} update report with `plan`, `files` and `blockers`.
 */
export async function update({
  gameId,
  path,
  packageId,
  allowNativeInjector,
  allowExecutables = [],
  runId,
}) {
  return invoke('update', {
    gameId,
    path,
    packageId,
    allowNativeInjector,
    allowExecutables,
    runId,
  });
}

/**
 * Operation history, newest first. Implemented by a later task.
 * @param {{gameId: string}} args
 * @returns {Promise<Array<{id: string, kind: string, game_id: string,
 *   installation: string, recorded_at: string, packages: Object[], undoes?: string}>>}
 */
export async function history({ gameId }) {
  return invoke('history', { gameId });
}

/**
 * Undo one operation.
 * @param {{operationId: string, allowNativeInjector?: boolean,
 *   allowExecutables?: string[], runId: string}} args
 * @returns {Promise<{kind: string, installed: Object[], removed: string[], undo: string}>}
 *   Rejects with `CommandError`; an approval refusal fills `approval` with the
 *   native injector and executable paths the ask must send back.
 */
export async function undo({
  operationId,
  allowNativeInjector = false,
  allowExecutables = [],
  runId,
}) {
  return invoke('undo', {
    operationId,
    allowNativeInjector,
    allowExecutables,
    runId,
  });
}

/**
 * Detect installed games without writing state. Implemented by a later task.
 * @returns {Promise<{installations: Object[], refused: Object[], skipped: Object[]}>}
 */
export async function detectGames() {
  return invoke('detect_games', {});
}

/**
 * Register a detected installation. Implemented by a later task.
 * @param {{gameId: string, path: string}} args @returns {Promise<Game>}
 */
export async function activateGame({ gameId, path }) {
  return invoke('activate_game', { gameId, path });
}

/**
 * Pick a game folder with a native picker, then register it. Later task.
 * @param {{gameId: string}} args @returns {Promise<Game|null>}
 */
export async function addGame({ gameId }) {
  return invoke('add_game', { gameId });
}

/**
 * Unregister an installation after a native confirmation. Later task.
 * @param {{gameId: string, path: string}} args @returns {Promise<Game>}
 */
export async function removeGame({ gameId, path }) {
  return invoke('remove_game', { gameId, path });
}

/**
 * Launch a registered installation. Later task.
 * @param {{gameId: string, path: string}} args
 * @returns {Promise<{game_id: string, installation: string, target: Object, warnings: string[]}>}
 */
export async function play({ gameId, path }) {
  return invoke('play', { gameId, path });
}

/**
 * Support text for Copy diagnostics. Later task.
 * @returns {Promise<string>}
 */
export async function diagnostics() {
  return invoke('diagnostics', {});
}

/**
 * Request cancellation of a run. Later task.
 * @param {{runId: string}} args @returns {Promise<void>}
 */
export async function cancelRun({ runId }) {
  return invoke('cancel_run', { runId });
}

/**
 * Subscribes to progress for one run. Later tasks emit the events.
 * @param {string} runId
 * @param {(event: ProgressEvent) => void} callback
 * @returns {Promise<() => void>} unlisten function
 */
export async function onProgress(runId, callback) {
  return window.__TAURI__.event.listen('modn://progress', (event) => {
    const payload = event.payload;
    if (payload && payload.runId === runId) {
      callback(payload.event);
    }
  });
}
