import assert from 'node:assert/strict';
import test from 'node:test';
import { createThemePanel } from '../../ui/js/theme/panel.js';
import {
  findAllByClass,
  findByTestId,
  deferred,
  flush,
  installDom,
} from './support/fake-dom.mjs';

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

function setup({ listThemes, selectAndApply, selectNoneAndApply, importAndRefresh } = {}) {
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
    selectNoneAndApply:
      selectNoneAndApply ||
      (async (game) => {
        calls.push(['none', game]);
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

test('Use default theme selects the explicit default and reloads the list', async () => {
  const { control, trigger, panel, calls } = setup();
  control.setGame('fixture');
  await open(control, trigger);
  findByTestId(panel, 'theme-reset').click();
  await flush();
  assert.ok(calls.some((call) => call.join('|') === 'none|fixture'));
  assert.equal(calls.filter((call) => call[0] === 'reset').length, 0);
  assert.equal(calls.filter((call) => call[0] === 'list').length, 2);
});

test('Use default theme shows no chosen theme after the reload', async () => {
  let lists = 0;
  const { control, trigger, panel } = setup({
    listThemes: async () => {
      lists += 1;
      const entries = themes();
      return {
        themes: lists === 1 ? entries : entries.map((theme) => ({ ...theme, selected: false })),
        diagnostics: [],
      };
    },
  });
  control.setGame('fixture');
  await open(control, trigger);
  assert.equal(findAllByClass(panel, 'modn-theme-pick--chosen').length, 1);
  findByTestId(panel, 'theme-reset').click();
  await flush();
  assert.equal(findAllByClass(panel, 'modn-theme-pick--chosen').length, 0);
  assert.match(panel.textContent, /Default theme/);
});

test('a pending selection disables every choice until it succeeds', async () => {
  const gate = deferred();
  let selections = 0;
  const { control, trigger, panel } = setup({
    selectAndApply: async () => {
      selections += 1;
      await gate.promise;
      return { applied: true, name: 'Daylight', source: { label: 'local registry' }, variant: '' };
    },
  });
  control.setGame('fixture');
  await open(control, trigger);
  findAllByClass(panel, 'modn-theme-pick')[1].click();
  findByTestId(panel, 'theme-use').click();
  for (const id of ['theme-use', 'theme-reset']) {
    assert.equal(findByTestId(panel, id).disabled, true, `${id} must be disabled`);
    assert.equal(findByTestId(panel, id).getAttribute('aria-disabled'), 'true');
  }
  assert.equal(findByTestId(panel, 'theme-variant').disabled, true);
  assert.equal(findByTestId(panel, 'theme-variant').getAttribute('aria-disabled'), 'true');
  for (const pick of findAllByClass(panel, 'modn-theme-pick')) {
    assert.equal(pick.disabled, true, 'a theme choice must be disabled');
    assert.equal(pick.getAttribute('aria-disabled'), 'true');
  }
  findByTestId(panel, 'theme-reset').click();
  assert.equal(selections, 1, 'a disabled default button starts no selection');
  gate.resolve();
  await flush();
  for (const id of ['theme-use', 'theme-reset']) {
    assert.equal(findByTestId(panel, id).disabled, false, `${id} must be enabled`);
    assert.equal(findByTestId(panel, id).getAttribute('aria-disabled'), 'false');
  }
  assert.equal(findByTestId(panel, 'theme-variant').disabled, false);
  for (const pick of findAllByClass(panel, 'modn-theme-pick')) {
    assert.equal(pick.disabled, false);
  }
});

test('a pending selection re-enables every choice after a failure', async () => {
  const gate = deferred();
  const { control, trigger, panel } = setup({
    selectNoneAndApply: async () => {
      await gate.promise;
      throw { code: 'BACKEND_FAILED' };
    },
  });
  control.setGame('fixture');
  await open(control, trigger);
  findByTestId(panel, 'theme-reset').click();
  assert.equal(findByTestId(panel, 'theme-reset').disabled, true);
  assert.equal(findByTestId(panel, 'theme-use').disabled, true);
  gate.reject({ code: 'BACKEND_FAILED' });
  await flush();
  assert.equal(findByTestId(panel, 'theme-reset').disabled, false);
  assert.equal(findByTestId(panel, 'theme-use').disabled, false);
  assert.match(panel.textContent, /could not start/);
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
