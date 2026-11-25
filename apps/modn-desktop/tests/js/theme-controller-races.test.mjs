import assert from 'node:assert/strict';
import test from 'node:test';
import {
  loadThemeForGame,
  resetAndApply,
  resetNoticeMemory,
  selectAndApply,
  selectNoneAndApply,
} from '../../ui/js/theme/controller.js';

const BASE = 'http://modn-theme.localhost/';

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

function applyStub(result = { applied: true, name: 'Night City', source: { label: 'main bucket' }, variant: 'neon' }) {
  const calls = [];
  const apply = (resolved) => {
    calls.push(resolved);
    return result;
  };
  apply.calls = calls;
  return apply;
}

test('a selection that finishes after navigation applies and resolves nothing', async () => {
  resetNoticeMemory();
  let settleSelect;
  const resolvedGames = [];
  const apply = applyStub();
  const api = {
    selectTheme: () =>
      new Promise((resolve) => {
        settleSelect = resolve;
      }),
    resolveTheme: async (game) => {
      resolvedGames.push(game);
      return answer({ game });
    },
  };
  const deps = { api, applyTheme: apply, toast: () => {} };
  await loadThemeForGame('first', deps);
  resolvedGames.length = 0;
  const pending = selectAndApply('first', 'night-city', 'neon', deps);
  await loadThemeForGame('second', deps);
  settleSelect({ selected: { game: 'first' } });
  const result = await pending;
  assert.equal(result.stale, true);
  assert.deepEqual(resolvedGames, ['second']);
  assert.equal(apply.calls.length, 2);
});

test('a reset that finishes after navigation applies and resolves nothing', async () => {
  resetNoticeMemory();
  let settleReset;
  const resolvedGames = [];
  const apply = applyStub();
  const api = {
    resetTheme: () =>
      new Promise((resolve) => {
        settleReset = resolve;
      }),
    resolveTheme: async (game) => {
      resolvedGames.push(game);
      return answer({ game });
    },
  };
  const deps = { api, applyTheme: apply, toast: () => {} };
  await loadThemeForGame('first', deps);
  resolvedGames.length = 0;
  const pending = resetAndApply('first', deps);
  await loadThemeForGame('second', deps);
  settleReset({ reset: ['first'] });
  const result = await pending;
  assert.equal(result.stale, true);
  assert.deepEqual(resolvedGames, ['second']);
  assert.equal(apply.calls.length, 2);
});

test('a none selection that finishes after navigation applies and resolves nothing', async () => {
  resetNoticeMemory();
  let settleSelect;
  const resolvedGames = [];
  const apply = applyStub();
  const api = {
    selectTheme: () =>
      new Promise((resolve) => {
        settleSelect = resolve;
      }),
    resolveTheme: async (game) => {
      resolvedGames.push(game);
      return answer({ game });
    },
  };
  const deps = { api, applyTheme: apply, toast: () => {} };
  await loadThemeForGame('first', deps);
  resolvedGames.length = 0;
  const pending = selectNoneAndApply('first', deps);
  await loadThemeForGame('second', deps);
  settleSelect({ selected: { game: 'first', theme: null } });
  const result = await pending;
  assert.equal(result.stale, true);
  assert.deepEqual(resolvedGames, ['second']);
  assert.equal(apply.calls.length, 2);
});

