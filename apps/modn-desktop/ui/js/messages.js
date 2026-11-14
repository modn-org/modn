const FALLBACK = 'Something went wrong. Try again, or copy diagnostics from Mod details.';

const MESSAGES = {
  BACKEND_FAILED: 'Modn could not start the operation. Check the backend path.',
  BACKEND_TIMEOUT: 'The operation took too long. Check diagnostics before trying again.',
  BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT: 'Done. Refresh to see the result.',
  BACKEND_STREAM_FAILED_AFTER_COMMIT: 'Done. Refresh to see the result.',
  PLAN_BLOCKED: 'Modn blocked this operation. Open details for the reason.',
  CONFLICT: 'Another package owns these files.',
  PACKAGE_NEEDED: 'Another mod depends on this package.',
  PACKAGE_PINNED: 'This package is pinned and cannot change.',
  DEPENDENCY_MISSING: 'A required mod is missing.',
  ALREADY_INSTALLED: 'This package is already installed.',
  ARCHIVE_INVALID: 'Modn cannot read this archive.',
  ARCHIVE_REQUIRED: 'Choose an archive first.',
  DIGEST_MISMATCH: 'The archive changed. Choose it again.',
  MOD_NOT_FOUND: 'Modn could not find this mod.',
  AMBIGUOUS_TARGET: 'Several mods match. Choose a more exact name.',
  APPROVAL_REQUIRED: 'This change needs your approval.',
  NATIVE_INJECTOR_NOT_ALLOWED: 'This mod runs native code and needs your approval.',
  PENDING_TRANSACTION: 'An earlier operation needs recovery.',
  LAUNCH_RECOVERY_PENDING: 'Recovery must finish before Modn can launch the game.',
  RECOVER_FAILED: 'Modn could not recover the earlier operation.',
  GAME_NOT_SELECTED: 'Choose one installation first.',
  GAME_FAILED: 'Modn could not use this game installation.',
  LAUNCH_INSTALLATION_UNREGISTERED: 'This installation is no longer registered.',
  LAUNCH_INSTALLATION_INVALID: 'Modn cannot launch this installation.',
  LAUNCH_GAME_MISMATCH: 'The selected installation belongs to another game.',
  LAUNCH_TARGET_UNAVAILABLE: 'Modn could not find the game executable.',
  CANCELLED: 'The operation was cancelled.',
  STATE_ERROR: 'Modn could not read or write its manager state.',
  IO_ERROR: 'Modn could not read or write a file.',
  GAME_MODULE_ERROR: 'The game module failed.',
  LIST_FAILED: 'Modn could not list owned packages.',
  INSTALL_FAILED: 'The install did not finish.',
  UNINSTALL_FAILED: 'The uninstall did not finish.',
  UPDATE_FAILED: 'The update did not finish.',
  HISTORY_FAILED: 'Modn could not read the operation history.',
  UNDO_NOT_FOUND: 'That operation is no longer available.',
  UNDO_CONFLICT: 'A newer operation blocks this undo.',
  UNDO_SOURCE_MANUAL: 'Modn cannot undo a manual source.',
  UNDO_FAILED: 'The undo did not finish.',
  DOCTOR_FAILED: 'Modn could not read diagnostics.',
};

/**
 * Maps a stable backend code to one short sentence. Unknown codes and raw
 * text fall back to a generic message; CLI output is never shown.
 * @param {{code?: string}|string|null} error
 * @returns {string}
 */
export function messageFor(error) {
  const code = error && typeof error === 'object' ? error.code : error;
  if (typeof code === 'string' && Object.prototype.hasOwnProperty.call(MESSAGES, code)) {
    return MESSAGES[code];
  }
  return FALLBACK;
}
