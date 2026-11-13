/**
 * Joins the bucket catalog with the manager-owned packages and filters the
 * result for the Library grid.
 */

/**
 * @typedef {Object} LibraryModel
 * @property {string} key
 * @property {string} modId
 * @property {string} name
 * @property {string} version
 * @property {string} bucket
 * @property {boolean} nativeInjector
 * @property {boolean} hasUpdate
 * @property {Object|null} installedPackage
 */

/**
 * @param {Object[]} entries catalog entries
 * @param {Object[]} packages owned packages
 * @returns {LibraryModel[]}
 */
export function mergeLibrary(entries, packages) {
  /** @type {Map<string, Object>} */
  const installed = new Map();
  /** @type {Object[]} */
  const manual = [];
  for (const pkg of packages) {
    const catalogMod = pkg.catalog && typeof pkg.catalog === 'object' ? pkg.catalog.mod_id : null;
    const modId = catalogMod || pkg.mod_id;
    if (modId) {
      installed.set(modId, pkg);
    } else {
      manual.push(pkg);
    }
  }
  /** @type {Set<string>} */
  const matched = new Set();
  /** @type {LibraryModel[]} */
  const models = entries.map((entry) => {
    const pkg = installed.get(entry.id) || null;
    if (pkg) {
      matched.add(entry.id);
    }
    return {
      key: `catalog-${entry.bucket || 'main'}-${entry.id}`,
      modId: entry.id,
      name: entry.name || entry.id,
      version: entry.version || '',
      bucket: entry.bucket || '',
      nativeInjector: Boolean(entry.native_injector),
      hasUpdate: Boolean(pkg && pkg.version && entry.version && pkg.version !== entry.version),
      installedPackage: pkg,
    };
  });
  for (const [modId, pkg] of installed) {
    if (matched.has(modId)) {
      continue;
    }
    const catalog = pkg.catalog && typeof pkg.catalog === 'object' ? pkg.catalog : null;
    models.push({
      key: `package-${pkg.id}`,
      modId,
      name: modId,
      version: pkg.version || '',
      bucket: catalog ? catalog.bucket : '',
      nativeInjector: false,
      hasUpdate: false,
      installedPackage: pkg,
    });
  }
  for (const pkg of manual) {
    models.push({
      key: `manual-${pkg.id}`,
      modId: pkg.mod_id || pkg.id,
      name: pkg.mod_id || pkg.id,
      version: pkg.version || '',
      bucket: '',
      nativeInjector: false,
      hasUpdate: false,
      installedPackage: pkg,
    });
  }
  models.sort((left, right) => left.name.localeCompare(right.name));
  return models;
}

/**
 * @param {LibraryModel[]} models
 * @param {'all'|'installed'|'available'} filter
 * @param {string} query
 * @returns {LibraryModel[]}
 */
export function visibleModels(models, filter, query) {
  const needle = query.trim().toLowerCase();
  return models.filter((model) => {
    if (filter === 'installed' && !model.installedPackage) {
      return false;
    }
    if (filter === 'available' && model.installedPackage) {
      return false;
    }
    return !needle || model.name.toLowerCase().includes(needle);
  });
}
