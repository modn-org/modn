import assert from 'node:assert/strict';
import test from 'node:test';

import { createTile } from '../../ui/js/library/tile.js';
import {
  findAllByClass,
  flush,
  installDom,
  installWindow,
} from './support/fake-dom.mjs';

const CTX = { gameId: 'demo', path: 'C:/game' };

/**
 * @param {string} key
 * @param {Object} pkg
 */
function installedModel(key, pkg) {
  return {
    key,
    modId: 'demo',
    name: 'Demo Mod',
    version: '1.0.0',
    bucket: 'main',
    nativeInjector: false,
    hasUpdate: false,
    installedPackage: pkg,
  };
}

test('a running uninstall survives a re-render that drops its catalog entry', async () => {
  installDom();
  const writing = new Promise(() => {});
  let runs = 0;
  installWindow({
    invoke: async (command) => {
      if (command === 'uninstall') {
        runs += 1;
        return writing;
      }
      return {};
    },
  });
  const pkg = {
    id: 'pkg-1',
    version: '1.0.0',
    catalog: { bucket: 'main', mod_id: 'demo' },
  };
  /** @type {Map<string, Object>} */
  const operations = new Map();
  const options = { onChanged: () => {}, operations };
  const catalogTile = createTile(
    installedModel('catalog-main-demo-pkg-1', pkg),
    CTX,
    options,
  );
  const running = findAllByClass(catalogTile, 'modn-op__button')[0];
  running.click();
  await flush();
  assert.equal(running.dataset.state, 'running');
  catalogTile.remove();

  const packageTile = createTile(installedModel('package-pkg-1', pkg), CTX, options);
  const buttons = findAllByClass(packageTile, 'modn-op__button');
  assert.equal(buttons.length, 1, 'the package view shows one Uninstall button');
  assert.equal(
    buttons[0],
    running,
    'the package view must reuse the running Uninstall of the catalog view',
  );
  assert.equal(buttons[0].dataset.state, 'running', 'no idle Uninstall may replace it');
  buttons[0].click();
  await flush();
  assert.equal(runs, 1, 'a click on the reused button must not start a second uninstall');
});
