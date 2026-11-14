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
 * Picks the installed package that came from the catalog entry's bucket.
 *
 * A package from another bucket never matches, and neither does a package
 * without a bucket, such as an install from a local archive. Unmatched
 * packages keep their own row.
 *
 * @param {Object[]} candidates
 * @param {string} bucket
 * @returns {Object|null}
 */
function pickPackage(candidates, bucket) {
  return (
    candidates.find(
      (pkg) => pkg.catalog && typeof pkg.catalog === 'object' && pkg.catalog.bucket === bucket,
    ) || null
  );
}

/**
 * @param {Object} pkg
 * @param {string} modId
 * @returns {LibraryModel}
 */
function packageModel(pkg, modId) {
  const catalog = pkg.catalog && typeof pkg.catalog === 'object' ? pkg.catalog : null;
  return {
    key: `package-${pkg.id}`,
    modId,
    name: modId,
    version: pkg.version || '',
    bucket: catalog ? catalog.bucket : '',
    nativeInjector: false,
    hasUpdate: false,
    installedPackage: pkg,
  };
}

/**
 * @param {Object[]} entries catalog entries
 * @param {Object[]} packages owned packages
 * @returns {LibraryModel[]}
 */
export function mergeLibrary(entries, packages) {
  /** @type {Map<string, Object[]>} */
  const installed = new Map();
  /** @type {Object[]} */
  const manual = [];
  for (const pkg of packages) {
    const catalogMod = pkg.catalog && typeof pkg.catalog === 'object' ? pkg.catalog.mod_id : null;
    const modId = catalogMod || pkg.mod_id;
    if (!modId) {
      manual.push(pkg);
      continue;
    }
    const list = installed.get(modId);
    if (list) {
      list.push(pkg);
    } else {
      installed.set(modId, [pkg]);
    }
  }
  /** @type {Set<Object>} */
  const matched = new Set();
  /** @type {LibraryModel[]} */
  const models = entries.map((entry) => {
    const bucket = entry.bucket || 'main';
    const candidates = (installed.get(entry.id) || []).filter((pkg) => !matched.has(pkg));
    const pkg = pickPackage(candidates, bucket);
    if (pkg) {
      matched.add(pkg);
    }
    return {
      key: pkg
        ? `catalog-${bucket}-${entry.id}-${pkg.id}`
        : `catalog-${bucket}-${entry.id}`,
      modId: entry.id,
      name: entry.name || entry.id,
      version: entry.version || '',
      bucket: entry.bucket || '',
      nativeInjector: Boolean(entry.native_injector),
      hasUpdate: Boolean(pkg && pkg.version && entry.version && pkg.version !== entry.version),
      installedPackage: pkg,
    };
  });
  for (const [modId, list] of installed) {
    for (const pkg of list) {
      if (!matched.has(pkg)) {
        models.push(packageModel(pkg, modId));
      }
    }
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
