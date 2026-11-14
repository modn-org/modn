import assert from 'node:assert/strict';
import test from 'node:test';

import { createRecentMenu } from '../../ui/js/library/recent.js';
import {
  findAllByClass,
  flush,
  findByTestId,
  installDom,
  installWindow,
} from './support/fake-dom.mjs';

const ENTRY = {
  id: 'op-install-1',
  kind: 'install',
  packages: [{ mod_id: 'cool-mod' }],
};

function installBackend() {
  installWindow({
    invoke: async (command) => {
      if (command === 'history') {
        return [ENTRY];
      }
      if (command === 'undo') {
        throw {
          code: 'APPROVAL_REQUIRED',
          approval: {
            nativeInjector: true,
            executables: ['C:/game/bin/patch.exe'],
            complete: true,
          },
        };
      }
      return {};
    },
  });
}

test('reopening Recent keeps the pending approval prompt of an entry', async () => {
  const document = installDom();
  installBackend();
  const menu = createRecentMenu({ gameId: 'game', onChanged() {}, undoHost: null });
  document.body.append(menu);
  const recent = findByTestId(menu, 'library-recent');
  recent.click();
  await flush();
  findByTestId(menu, 'recent-undo').click();
  await flush();
  assert.ok(findByTestId(menu, 'ask-allow'), 'the ask renders for the Undo refusal');
  assert.match(menu.textContent, /C:\/game\/bin\/patch\.exe/);
  recent.click();
  assert.equal(menu.querySelector('.modn-recent__menu').hidden, true);
  recent.click();
  await flush();
  assert.equal(
    findAllByClass(menu, 'modn-ask__actions').length,
    1,
    'the reloaded row reattaches the same pending ask once',
  );
  assert.ok(findByTestId(menu, 'ask-allow'), 'Allow is still reachable after the reload');
  assert.match(menu.textContent, /C:\/game\/bin\/patch\.exe/);
});

test('an entry without a pending ask does not carry one into a reload', async () => {
  const document = installDom();
  installWindow({
    invoke: async (command) => (command === 'history' ? [ENTRY] : {}),
  });
  const menu = createRecentMenu({ gameId: 'game', onChanged() {}, undoHost: null });
  document.body.append(menu);
  const recent = findByTestId(menu, 'library-recent');
  recent.click();
  await flush();
  recent.click();
  recent.click();
  await flush();
  assert.equal(findByTestId(menu, 'ask-allow'), null);
  assert.ok(findByTestId(menu, 'recent-undo'), 'the row still offers Undo');
});
