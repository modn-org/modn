import assert from 'node:assert/strict';
import test from 'node:test';

import { createTile, createUndoLauncher } from '../../ui/js/library/tile.js';
import {
  findByTestId,
  flush,
  installDom,
  installWindow,
  lastToast,
  toastHost,
} from './support/fake-dom.mjs';

/**
 * Installs a toast Undo host and returns its launcher.
 * @param {(args: Object) => Promise<unknown>} runUndo
 * @param {() => void} [onSettled]
 */
function makeLauncher(runUndo, onSettled = () => {}) {
  const document = installDom();
  const host = document.createElement('div');
  document.body.append(host);
  const launch = createUndoLauncher({ host, runUndo, onSettled });
  return { host, launch };
}

test('toast A undoes operation A after operation B commits on the same package', async () => {
  installDom();
  const undoHost = document.createElement('div');
  document.body.append(undoHost);
  const undone = [];
  let commits = 0;
  installWindow({
    invoke: async (command, args) => {
      if (command === 'undo') {
        undone.push(args);
        return {};
      }
      if (command === 'uninstall') {
        commits += 1;
        return { operationId: `op-${commits}` };
      }
      return {};
    },
  });
  const pkg = {
    id: 'pkg-1',
    version: '1.0.0',
    catalog: { bucket: 'main', mod_id: 'demo' },
  };
  const model = {
    key: 'package-pkg-1',
    modId: 'demo',
    name: 'Demo Mod',
    version: '1.0.0',
    bucket: 'main',
    nativeInjector: false,
    hasUpdate: false,
    installedPackage: pkg,
  };
  const tile = createTile(
    model,
    { gameId: 'demo', path: 'C:/game' },
    { onChanged: () => {}, operations: new Map(), undoHost },
  );
  document.body.append(tile);
  const button = findByTestId(tile, 'op-button');
  button.click();
  await flush();
  button.click();
  await flush();
  assert.equal(commits, 2, 'two operations commit on one package');
  const toasts = toastHost().children;
  assert.equal(toasts.length, 2, 'each commit keeps its own toast');

  findByTestId(toasts[0], 'toast-undo').click();
  await flush();

  assert.equal(undone.length, 1);
  assert.equal(
    undone[0].operationId,
    'op-1',
    'toast A must undo operation A, not the newer operation B',
  );
});

test('a blocked toast Undo shows its reason then removes its temporary row', async () => {
  installWindow();
  const calls = [];
  const { host, launch } = makeLauncher(async (args) => {
    calls.push(args);
    throw { code: 'CONFLICT', message: 'another package owns these files' };
  });

  launch('op-blocked');
  await flush();

  assert.equal(calls.length, 1);
  const toast = lastToast();
  assert.ok(toast, 'a blocked Undo reports its reason');
  assert.equal(
    toast.textContent,
    'Another package owns these files.',
    'the short reason stays after the temporary row leaves',
  );
  assert.equal(
    host.childElementCount,
    0,
    'a blocked Undo must not leave its temporary row in the Library',
  );
});

test('a declined ask removes its row and a retry keeps the operation ID', async () => {
  installWindow();
  const calls = [];
  const { host, launch } = makeLauncher(async (args) => {
    calls.push(args);
    if (calls.length === 1) {
      throw {
        code: 'APPROVAL_REQUIRED',
        approval: { nativeInjector: true, executables: [], complete: false },
      };
    }
    return { operationId: 'op-undo' };
  });

  launch('op-declined');
  await flush();

  const undo = findByTestId(host, 'toast-undo-button');
  assert.ok(undo, 'the temporary row offers an Undo button');
  assert.ok(findByTestId(host, 'ask-decline'), 'the approval prompt waits for a decision');
  findByTestId(host, 'ask-decline').click();
  await flush();

  assert.equal(calls.length, 1, 'a declined ask does not run the write');
  assert.equal(
    host.childElementCount,
    0,
    'a declined ask must not leave its temporary row in the Library',
  );

  undo.click();
  await flush();

  assert.equal(calls.length, 2, 'the idle Undo button still retries');
  assert.equal(
    calls[1].operationId,
    'op-declined',
    'a retry must undo the same operation the toast bound',
  );
});

test('a cancelled toast Undo removes its temporary row', async () => {
  installWindow();
  let settled = 0;
  const { host, launch } = makeLauncher(
    async () => {
      throw { code: 'CANCELLED', message: 'cancelled' };
    },
    () => {
      settled += 1;
    },
  );

  launch('op-cancelled');
  await flush();

  assert.equal(settled, 1, 'a cancelled Undo still refreshes the view');
  assert.equal(lastToast().textContent, 'Cancelled. Nothing changed.');
  assert.equal(
    host.childElementCount,
    0,
    'a cancelled Undo must not leave its temporary row in the Library',
  );
});
