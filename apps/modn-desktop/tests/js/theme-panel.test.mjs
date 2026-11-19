import assert from 'node:assert/strict';
import test from 'node:test';
import { createThemePanel } from '../../ui/js/theme/panel.js';
import {
  findAllByClass,
  findByTestId,
  flush,
  installDom,
} from './support/fake-dom.mjs';

/** Two themes with different variants and sources. */
function themes() {
  return [
    {
      id: 'night-city',
      name: 'Night City',
      variant: 'neon',
      variants: ['neon', 'noir'],
      selected: true,
      source: { kind: 'official', bucket: 'main', label: 'main bucket' },
    },
    {
      id: 'daylight',
      name: 'Daylight',
      variant: null,
      variants: ['bright'],
      selected: false,
      source: { kind: 'local', bucket: null, label: 'local registry' },
    },
  ];
}

/** Installs a fake document and builds one control with stubs. */
function setup({ listThemes, selectAndApply, resetAndApply, importAndRefresh } = {}) {
  installDom();
  const host = document.createElement('div');
  document.body.append(host);
  const calls = [];
  const api = {
    listThemes: async (game) => {
      calls.push(['list', game]);
      if (listThemes) {
        return listThemes(game);
      }
      return { themes: themes(), diagnostics: [] };
    },
  };
  const control = createThemePanel(host, {
    api,
    loadThemeForGame: async () => ({ applied: false }),
    selectAndApply:
      selectAndApply ||
      (async (game, theme, variant) => {
        calls.push(['select', game, theme, variant]);
        return { applied: true, name: theme, source: { label: 'local registry' }, variant };
      }),
    resetAndApply:
      resetAndApply ||
      (async (game) => {
        calls.push(['reset', game]);
        return { applied: false };
      }),
    importAndRefresh:
      importAndRefresh ||
      (async (game) => {
        calls.push(['import', game]);
        return { registered: { id: 'night-city' } };
      }),
  });
  const trigger = findByTestId(control.element, 'theme-button');
  const panel = document.getElementById('modn-theme-panel');
  return { control, trigger, panel, calls };
}

/** Opens the panel and waits for the list. */
async function open(control, trigger) {
  trigger.click();
  await flush();
}

test('the trigger starts collapsed', () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  assert.equal(trigger.textContent, 'Theme');
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  assert.equal(panel.hidden, true);
});

test('opening lists the themes and shows the current name and source label', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  control.setTheme({
    applied: true,
    name: 'Night City',
    source: { label: 'main bucket' },
    variant: 'neon',
  });
  await open(control, trigger);
  assert.equal(panel.hidden, false);
  assert.equal(trigger.getAttribute('aria-expanded'), 'true');
  assert.match(panel.textContent, /Night City/);
  assert.match(panel.textContent, /main bucket/);
  assert.ok(findByTestId(panel, 'theme-list'));
});

test('shows the default theme when no theme is applied', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  control.setTheme({ applied: false });
  await open(control, trigger);
  assert.match(panel.textContent, /Default theme/);
});

test('every theme shows its source label', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  const picks = findAllByClass(panel, 'modn-theme-pick');
  assert.equal(picks.length, 2);
  assert.match(picks[0].textContent, /Night City/);
  assert.match(picks[0].textContent, /main bucket/);
  assert.match(picks[1].textContent, /Daylight/);
  assert.match(picks[1].textContent, /local registry/);
});

test('marks the selected theme once', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  const marks = findAllByClass(panel, 'modn-theme-mark');
  assert.equal(marks.length, 1);
  assert.equal(marks[0].textContent, 'Selected');
});

test('offers no variant plus every variant of the chosen theme', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  const select = findByTestId(panel, 'theme-variant');
  const values = select.children.map((option) => option.value);
  assert.deepEqual(values, ['', 'neon', 'noir']);
});

test('switching the chosen theme switches its variants', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  findAllByClass(panel, 'modn-theme-pick')[1].click();
  const select = findByTestId(panel, 'theme-variant');
  assert.deepEqual(
    select.children.map((option) => option.value),
    ['', 'bright'],
  );
});

test('Use theme sends the chosen theme and variant', async () => {
  const { control, trigger, panel, calls } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  findAllByClass(panel, 'modn-theme-pick')[1].click();
  const select = findByTestId(panel, 'theme-variant');
  select.value = 'bright';
  select.listeners.get('change')[0]();
  findByTestId(panel, 'theme-use').click();
  await flush();
  assert.ok(calls.some((call) => call.join('|') === 'select|fixture|daylight|bright'));
  assert.equal(calls.filter((call) => call[0] === 'list').length, 2);
});

test('Use default theme resets and reloads the list', async () => {
  const { control, trigger, panel, calls } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  findByTestId(panel, 'theme-reset').click();
  await flush();
  assert.ok(calls.some((call) => call.join('|') === 'reset|fixture'));
  assert.equal(calls.filter((call) => call[0] === 'list').length, 2);
});

test('Import theme reloads the list after a registration', async () => {
  const { control, trigger, panel, calls } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  findByTestId(panel, 'theme-import').click();
  await flush();
  assert.ok(calls.some((call) => call[0] === 'import'));
  assert.equal(calls.filter((call) => call[0] === 'list').length, 2);
});

test('import diagnostics are listed one rule per line', async () => {
  const { control, trigger, panel } = setup({
    importAndRefresh: async () => ({
      diagnostics: [
        { rule: 'unknown-property', message: 'property "transform" is not allowed' },
        { rule: 'css-syntax', message: 'unexpected token' },
      ],
    }),
  });
  control.setGame('fixture');
  await open(control, trigger);
  findByTestId(panel, 'theme-import').click();
  await flush();
  const lines = findAllByClass(panel, 'modn-theme-diagnostic');
  assert.deepEqual(
    lines.map((item) => item.textContent),
    ['unknown-property: property "transform" is not allowed', 'css-syntax: unexpected token'],
  );
});

test('a failing list command shows one unavailable line and hides the controls', async () => {
  const { control, trigger, panel } = setup({
    listThemes: async () => {
      throw { code: 'BACKEND_FAILED' };
    },
  });
  control.setGame('fixture');
  await open(control, trigger);
  assert.match(panel.textContent, /^Themes are unavailable: /);
  assert.equal(findByTestId(panel, 'theme-use'), null);
  assert.equal(findByTestId(panel, 'theme-list'), null);
});

test('the default theme flag turns the session off and disables the buttons', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  control.setTheme({ applied: false, disabled: 'default-theme-flag' });
  await open(control, trigger);
  assert.match(panel.textContent, /Themes are off for this session/);
  for (const id of ['theme-use', 'theme-reset', 'theme-import']) {
    assert.equal(findByTestId(panel, id).disabled, true, `${id} must be disabled`);
  }
});

test('Escape closes the panel and returns focus to the trigger', async () => {
  const { control, trigger, panel } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  let prevented = false;
  const event = { key: 'Escape', preventDefault: () => { prevented = true; } };
  for (const listener of control.element.listeners.get('keydown')) {
    listener(event);
  }
  assert.equal(prevented, true);
  assert.equal(panel.hidden, true);
  assert.equal(trigger.getAttribute('aria-expanded'), 'false');
  assert.equal(document.activeElement, trigger);
});

test('without an active game the panel asks for one', async () => {
  const { control, trigger, panel, calls } = setup();
  await open(control, trigger);
  assert.match(panel.textContent, /Open a game to choose its theme/);
  assert.equal(calls.filter((call) => call[0] === 'list').length, 0);
});
