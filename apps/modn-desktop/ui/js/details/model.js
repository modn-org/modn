/**
 * Data shaping for the Mod details view. It joins one catalog entry with the
 * manager-owned package of the same mod and filters the operation history.
 */

/** @type {Record<string, string>} */
const SOURCE_LABELS = {
  'github-release': 'GitHub release',
  manual: 'local archive',
  url: 'direct download',
};

/** @type {Record<string, string>} */
const KIND_LABELS = {
  install: 'Installed',
  update: 'Updated',
  uninstall: 'Uninstalled',
  undo: 'Undid',
};

/**
 * @param {unknown} value
 * @returns {string}
 */
function text(value) {
  return typeof value === 'string' ? value : '';
}

/**
 * @param {Object|null} source
 * @returns {string}
 */
function sourceLabel(source) {
  if (!source || typeof source !== 'object' || typeof source.kind !== 'string') {
    return '';
  }
  return SOURCE_LABELS[source.kind] || source.kind.replace(/[-_]+/g, ' ');
}

/**
 * Finds the owned package whose catalog or stored id is the requested mod.
 * @param {Object[]} packages
 * @param {string} modId
 * @returns {Object|null}
 */
export function installedOf(packages, modId) {
  if (!Array.isArray(packages)) {
    return null;
  }
  const found = packages.find((pkg) => {
    if (!pkg || typeof pkg !== 'object') {
      return false;
    }
    const catalog = pkg.catalog && typeof pkg.catalog === 'object' ? pkg.catalog.mod_id : null;
    return (text(catalog) || text(pkg.mod_id)) === modId;
  });
  return found || null;
}

/**
 * @param {string} modId
 * @param {Object[]} entries
 * @param {Object[]} packages
 * @returns {{entry: Object|null, installed: Object|null, hasUpdate: boolean}}
 */
export function modelOf(modId, entries, packages) {
  const entry = Array.isArray(entries)
    ? entries.find((item) => item && typeof item === 'object' && item.id === modId) || null
    : null;
  const installed = installedOf(packages, modId);
  const entryVersion = entry ? text(entry.version) : '';
  const installedVersion = installed ? text(installed.version) : '';
  return {
    entry,
    installed,
    hasUpdate: Boolean(entryVersion && installedVersion && entryVersion !== installedVersion),
  };
}

/**
 * @param {{entry: Object|null, installed: Object|null}} model
 * @returns {string}
 */
export function versionOf(model) {
  if (model.entry && text(model.entry.version)) {
    return text(model.entry.version);
  }
  return model.installed ? text(model.installed.version) : '';
}

/**
 * @param {{entry: Object|null}} model
 * @returns {string}
 */
export function bucketOf(model) {
  return model.entry ? text(model.entry.bucket) : '';
}

/**
 * Short description built from catalog facts. A bucket may set its own
 * description; otherwise the bucket and source kind describe the mod.
 * @param {Object|null} entry
 * @returns {string}
 */
export function describeMod(entry) {
  if (!entry || typeof entry !== 'object') {
    return 'This mod is not in the bucket catalog.';
  }
  if (text(entry.description).trim()) {
    return text(entry.description).trim();
  }
  const bucket = text(entry.bucket);
  const placement = bucket ? `from the ${bucket} bucket` : 'from the bucket catalog';
  const source = sourceLabel(entry.source);
  return source
    ? `Available ${placement} via ${source}.`
    : `Available ${placement}.`;
}

/**
 * @param {{entry: Object|null, installed: Object|null, hasUpdate: boolean}} model
 * @returns {string}
 */
export function stateLabel(model) {
  if (!model.installed) {
    return 'Not installed';
  }
  const version = text(model.installed.version);
  const base = version ? `Installed ${version}` : 'Installed';
  return model.hasUpdate ? `${base} · Update available` : base;
}

/**
 * Keeps the history entries that touch this mod, newest first.
 * @param {Object[]} entries
 * @param {string} modId
 * @param {string} [packageId]
 * @returns {Object[]}
 */
export function historyForMod(entries, modId, packageId) {
  if (!Array.isArray(entries)) {
    return [];
  }
  const concerning = entries.filter((entry) => {
    if (!entry || typeof entry !== 'object' || !Array.isArray(entry.packages)) {
      return false;
    }
    return entry.packages.some((pkg) => {
      if (!pkg || typeof pkg !== 'object') {
        return false;
      }
      if (text(pkg.mod_id) === modId) {
        return true;
      }
      return Boolean(
        packageId &&
          (text(pkg.package_id_before) === packageId ||
            text(pkg.package_id_after) === packageId),
      );
    });
  });
  return concerning.sort((left, right) =>
    text(right.recorded_at).localeCompare(text(left.recorded_at)),
  );
}

/**
 * @param {Object} entry
 * @returns {string}
 */
export function undoLabel(entry) {
  const kind = text(entry && entry.kind);
  const label = KIND_LABELS[kind] || 'Changed';
  const pkg = entry && Array.isArray(entry.packages) ? entry.packages[0] : null;
  const release = pkg && pkg.release_after && typeof pkg.release_after === 'object'
    ? pkg.release_after
    : null;
  const version = release ? text(release.version) : '';
  return version ? `${label} ${version}` : label;
}

/**
 * @param {string} value recorded timestamp
 * @returns {string}
 */
export function formatWhen(value) {
  const time = Date.parse(text(value));
  if (!Number.isFinite(time)) {
    return '';
  }
  return new Date(time).toLocaleString();
}
