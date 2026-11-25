function uncoveredIn(value) {
  if (!value || typeof value !== 'object') {
    return null;
  }
  const direct = value.uncovered;
  if (direct && typeof direct === 'object') {
    return direct;
  }
  const approval = value.approval;
  if (approval && typeof approval === 'object' && approval.uncovered) {
    return approval.uncovered;
  }
  return null;
}

function refusalIn(refusal) {
  if (!refusal || typeof refusal !== 'object') {
    return null;
  }
  return {
    native_injector: refusal.nativeInjector === true,
    executables: Array.isArray(refusal.executables) ? refusal.executables : [],
  };
}

function executablePaths(value) {
  if (!value || !Array.isArray(value.executables)) {
    return [];
  }
  return value.executables.filter((path) => typeof path === 'string' && path.length > 0);
}

/**
 * @param {unknown} err
 * @param {string} code
 * @param {(() => Promise<Object|null>)|null} plan
 * @returns {Promise<{native: boolean, executables: string[]}>}
 */
export async function approvalsFrom(err, code, plan) {
  const refusal = err && typeof err === 'object' ? err.approval : null;
  const uncovered = uncoveredIn(err) || refusalIn(refusal);
  let native = Boolean(uncovered && uncovered.native_injector)
    || code === 'NATIVE_INJECTOR_NOT_ALLOWED';
  let executables = executablePaths(uncovered);
  if (plan) {
    let planned = null;
    try {
      planned = uncoveredIn(await plan());
    } catch {
      planned = null;
    }
    if (planned) {
      native = native || planned.native_injector === true;
      const merged = new Set(executables);
      for (const path of executablePaths(planned)) {
        merged.add(path);
      }
      executables = [...merged];
    }
  }
  return { native, executables };
}
