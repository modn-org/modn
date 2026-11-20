import assert from 'node:assert/strict';
import test from 'node:test';
import {
  clearTheme,
  importAndRefresh,
  loadThemeForGame,
  resetAndApply,
  selectAndApply,
} from '../../ui/js/theme/controller.js';

const BASE = 'http://modn-theme.localhost/';

/** A resolve answer that applies cleanly. */
function answer(overrides = {}) {
  return {
    game: 'fixture',
    asset_base: BASE,
    theme: {
      id: 'night-city',
      name: 'Night City',
      source: { kind: 'official', bucket: 'main', label: 'main bucket' },
      variant: 'neon',
      css: '.modn-app{}',
      variant_css: null,
    },
    fallback: null,
    diagnostics: [],
    ...overrides,
  };
}

/** An apply stub that records the answer and returns a fixed result. */
function applyStub(result = { applied: true, name: 'Night City', source: { label: 'main bucket' }, variant: 'neon' }) {
  const calls = [];
  const apply = (resolved) => {
    calls.push(resolved);
    return result;
  };
  apply.calls = calls;
  return apply;
}

test('applies the resolve answer and returns the theme identity', async () => {
  const apply = applyStub();
  const result = await loadThemeForGame('fixture', {
    api: { resolveTheme: async () => answer() },
    applyTheme: apply,
    toast: () => assert.fail('no toast for a clean answer'),
  });
  assert.equal(apply.calls.length, 1);
  assert.equal(result.applied, true);
  assert.equal(result.name, 'Night City');
  assert.equal(result.source.label, 'main bucket');
  assert.equal(result.unavailable, false);
});

test('shows one toast with the first rule and its path', async () => {
  const toasts = [];
  const diagnostics = [
    { rule: 'unknown-property', path: 'theme.css', message: 'property "transform" is not allowed' },
    { rule: 'remote-url', path: 'theme.css', message: 'a remote url is not allowed' },
  ];
  await loadThemeForGame('fixture', {
    api: {
      resolveTheme: async () =>
        answer({ fallback: { reason: 'theme-invalid', message: 'the selection is invalid' }, diagnostics }),
    },
    applyTheme: applyStub(),
    toast: (text) => toasts.push(text),
  });
  assert.deepEqual(toasts, [
    'Theme not applied: unknown-property in theme.css: property "transform" is not allowed',
  ]);
});

test('shows one toast for a fallback without a diagnostic', async () => {
  const toasts = [];
  await loadThemeForGame('fixture', {
    api: {
      resolveTheme: async () =>
        answer({ fallback: { reason: 'theme-missing', message: 'the theme is gone' } }),
    },
    applyTheme: applyStub(),
    toast: (text) => toasts.push(text),
  });
  assert.deepEqual(toasts, ['Theme not applied: theme-missing: the theme is gone']);
});

test('a clean answer shows no toast', async () => {
  const toasts = [];
  await loadThemeForGame('fixture', {
    api: { resolveTheme: async () => answer() },
    applyTheme: applyStub(),
    toast: (text) => toasts.push(text),
  });
  assert.deepEqual(toasts, []);
});

test('shows one toast when the stylesheet cannot be applied', async () => {
  const toasts = [];
  await loadThemeForGame('fixture', {
    api: { resolveTheme: async () => answer() },
    applyTheme: () => ({ applied: false, error: new Error('the stylesheet is invalid') }),
    toast: (text) => toasts.push(text),
  });
  assert.deepEqual(toasts, ['Theme not applied: the stylesheet is invalid']);
});

test('a failing command reports unavailable, applies nothing and shows no toast', async () => {
  const apply = applyStub();
  const toasts = [];
  const result = await loadThemeForGame('fixture', {
    api: {
      resolveTheme: async () => {
        throw { code: 'BACKEND_FAILED', message: 'The CLI was not found' };
      },
    },
    applyTheme: apply,
    toast: (text) => toasts.push(text),
  });
  assert.equal(apply.calls.length, 0);
  assert.deepEqual(toasts, []);
  assert.equal(result.applied, false);
  assert.equal(result.unavailable, true);
  assert.equal(result.message, 'Modn could not start the operation. Check the backend path.');
});

test('selectAndApply selects before it resolves and applies', async () => {
  const calls = [];
  const api = {
    selectTheme: async (game, theme, variant) => {
      calls.push(['select', game, theme, variant]);
      return { selected: { game, theme, variant } };
    },
    resolveTheme: async (game) => {
      calls.push(['resolve', game]);
      return answer();
    },
  };
  await selectAndApply('fixture', 'night-city', 'neon', {
    api,
    applyTheme: applyStub(),
    toast: () => {},
  });
  assert.deepEqual(calls, [
    ['select', 'fixture', 'night-city', 'neon'],
    ['resolve', 'fixture'],
  ]);
});

test('selectAndApply sends an empty variant as absent', async () => {
  const calls = [];
  await selectAndApply('fixture', 'night-city', '', {
    api: {
      selectTheme: async (game, theme, variant) => calls.push(variant),
      resolveTheme: async () => answer(),
    },
    applyTheme: applyStub(),
    toast: () => {},
  });
  assert.deepEqual(calls, [undefined]);
});

test('resetAndApply resets before it resolves and applies', async () => {
  const calls = [];
  await resetAndApply('fixture', {
    api: {
      resetTheme: async (game) => {
        calls.push(['reset', game]);
        return { reset: [game] };
      },
      resolveTheme: async (game) => {
        calls.push(['resolve', game]);
        return answer();
      },
    },
    applyTheme: applyStub(),
    toast: () => {},
  });
  assert.deepEqual(calls, [
    ['reset', 'fixture'],
    ['resolve', 'fixture'],
  ]);
});

test('importAndRefresh returns a refusal that carries diagnostics', async () => {
  const refusal = {
    code: 'THEME_INVALID',
    message: 'the theme is not valid',
    diagnostics: [{ rule: 'remote-url', message: 'a remote url is not allowed' }],
  };
  const result = await importAndRefresh('fixture', {
    api: {
      importTheme: async () => {
        throw refusal;
      },
    },
  });
  assert.equal(result, refusal);
  assert.equal(result.diagnostics.length, 1);
});

test('importAndRefresh returns a registered answer unchanged', async () => {
  const registered = { registered: { id: 'night-city' }, digest: 'blake3:abc' };
  const result = await importAndRefresh('fixture', { api: { importTheme: async () => registered } });
  assert.equal(result, registered);
});

test('the answer of an earlier game is ignored', async () => {
  const apply = applyStub();
  const toasts = [];
  let settleFirst;
  const first = new Promise((resolve) => {
    settleFirst = resolve;
  });
  const api = {
    resolveTheme: (game) => (game === 'first' ? first : Promise.resolve(answer({ game: 'second' }))),
  };
  const deps = { api, applyTheme: apply, toast: (text) => toasts.push(text) };
  const firstLoad = loadThemeForGame('first', deps);
  await loadThemeForGame('second', deps);
  settleFirst(answer({ game: 'first', diagnostics: [{ rule: 'unknown-hook', path: 'theme.css', message: 'x' }] }));
  const result = await firstLoad;
  assert.equal(result.applied, false);
  assert.equal(result.stale, true);
  assert.equal(apply.calls.length, 1);
  assert.deepEqual(toasts, []);
});

test('the default theme flag reports disabled without a toast', async () => {
  const toasts = [];
  const result = await loadThemeForGame('fixture', {
    api: {
      resolveTheme: async () => ({
        game: 'fixture',
        theme: null,
        fallback: null,
        disabled: 'default-theme-flag',
        diagnostics: [],
        asset_base: BASE,
      }),
    },
    applyTheme: () => ({ applied: false, error: null }),
    toast: (text) => toasts.push(text),
  });
  assert.equal(result.disabled, 'default-theme-flag');
  assert.equal(result.applied, false);
  assert.deepEqual(toasts, []);
});

test('clearTheme drops a pending answer and applies the default theme', async () => {
  let settle;
  const pending = new Promise((resolve) => {
    settle = resolve;
  });
  const applied = [];
  const apply = (resolved) => {
    applied.push(resolved);
    return { applied: false, error: null };
  };
  const loading = loadThemeForGame('fixture', {
    api: { resolveTheme: () => pending },
    applyTheme: apply,
    toast: () => {},
  });
  clearTheme({ applyTheme: apply });
  settle(answer());
  const result = await loading;
  assert.equal(result.stale, true);
  assert.deepEqual(applied, [{ theme: null }]);
  assert.deepEqual(clearTheme({ applyTheme: apply }), { applied: false, error: null });
});
