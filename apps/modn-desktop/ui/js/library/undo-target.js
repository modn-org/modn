/**
 * Binds a finished operation to the history record it committed, so Undo
 * never reverses another mod's operation.
 */

/**
 * Reads the history operation ID a write result carries. Only the top-level
 * `operationId` counts: a result without one gets a plain toast, because
 * guessing from package IDs can bind the toast to a later operation.
 * @param {unknown} result
 * @returns {string}
 */
export function operationIdOf(result) {
  if (!result || typeof result !== 'object') {
    return '';
  }
  return typeof result.operationId === 'string' ? result.operationId : '';
}
