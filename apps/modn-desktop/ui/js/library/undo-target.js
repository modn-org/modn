/**
 * Binds a finished operation to the history record it committed, so Undo
 * never reverses another mod's operation.
 */

/**
 * Collects the package and mod IDs that name the operation a result finished.
 * @param {unknown} result
 * @returns {{packageIds: string[], modIds: string[]}}
 */
export function operationTarget(result) {
  const packageIds = new Set();
  const modIds = new Set();
  /** @param {unknown} value @param {Set<string>} ids */
  const add = (value, ids) => {
    if (typeof value === 'string' && value) {
      ids.add(value);
    }
  };
  /** @param {unknown} value */
  const addPackage = (value) => {
    if (!value || typeof value !== 'object') {
      return;
    }
    add(value.id, packageIds);
    add(value.package_id, packageIds);
    add(value.mod_id, modIds);
    if (value.catalog && typeof value.catalog === 'object') {
      add(value.catalog.mod_id, modIds);
    }
  };
  if (result && typeof result === 'object') {
    addPackage(result);
    addPackage(result.installed);
    for (const key of ['packages', 'removed', 'installed']) {
      for (const item of Array.isArray(result[key]) ? result[key] : []) {
        add(item, packageIds);
      }
    }
    add(result.package_id, packageIds);
    add(result.uninstalled, packageIds);
    if (typeof result.package === 'string' && result.package.includes('/')) {
      add(result.package.slice(result.package.lastIndexOf('/') + 1), modIds);
    }
  }
  return { packageIds: [...packageIds], modIds: [...modIds] };
}

/**
 * True when a history entry changed one of the target's packages or mods.
 * Package IDs win because the same mod can be installed twice; the mod ID is
 * only a fallback for a result that carries no package ID.
 * @param {Object|null} entry
 * @param {{packageIds: string[], modIds: string[]}} target
 * @returns {boolean}
 */
export function operationMatches(entry, target) {
  if (!entry || typeof entry !== 'object') {
    return false;
  }
  const packages = Array.isArray(entry.packages) ? entry.packages : [];
  const byPackage = target.packageIds.length > 0;
  return packages.some((pkg) => pkg && (byPackage
    ? target.packageIds.includes(pkg.package_id_after)
      || target.packageIds.includes(pkg.package_id_before)
    : target.modIds.includes(pkg.mod_id)));
}

/**
 * Binds the toast to the operation this run committed. A result that carries
 * its own ID wins; otherwise the matching history entry supplies it. An
 * unknown target means no Undo action, never the newest operation of the game.
 * @param {unknown} result
 * @param {((match: Object) => Promise<Object|null>)|null} newest
 * @returns {Promise<string>}
 */
export async function operationIdOf(result, newest) {
  for (const key of ['operation_id', 'operationId']) {
    if (result && typeof result[key] === 'string' && result[key]) {
      return result[key];
    }
  }
  const target = operationTarget(result);
  if (!newest || (target.packageIds.length === 0 && target.modIds.length === 0)) {
    return '';
  }
  try {
    const entry = await newest(target);
    return entry && operationMatches(entry, target) && typeof entry.id === 'string'
      ? entry.id
      : '';
  } catch {
    return '';
  }
}
