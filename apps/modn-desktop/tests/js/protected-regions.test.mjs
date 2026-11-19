import assert from 'node:assert/strict';
import test from 'node:test';

import { createPlanSection } from '../../ui/js/details/plan.js';
import { renderAsk } from '../../ui/js/library/ask.js';
import { findAllByClass, flush, installDom, installWindow } from './support/fake-dom.mjs';

const REPORT = {
  plan: {
    operations: [
      { destination: 'C:/game/mods/one.lua', kind: 'write' },
      { destination: 'C:/game/bin/old.dll', kind: 'remove' },
    ],
  },
  approval: {
    required: true,
    uncovered: { native_injector: true, executables: ['C:/game/bin/patch.exe'] },
  },
  blockers: [{ code: 'UNKNOWN_BLOCKER' }],
};

const PERMISSIONS = {
  native: true,
  executables: ['C:/game/bin/patch.exe'],
};

/** Fires the listeners of one event type on a fake node. */
function fire(node, type) {
  for (const listener of [...(node.listeners.get(type) || [])]) {
    listener.call(node, { type, target: node });
  }
}

/** Returns every class token on an element and its descendants. */
function classTokens(root) {
  const tokens = [];
  const visit = (node) => {
    const names = node.className.split(/\s+/).filter(Boolean);
    assert.ok(
      names.length > 0,
      `${node.tagName.toLowerCase()} inside the region carries no hook class`,
    );
    for (const name of names) {
      tokens.push([node, name]);
    }
    for (const child of node.children) {
      visit(child);
    }
  };
  visit(root);
  return tokens;
}

/** Asserts that every class inside the region starts with its hook prefix. */
function assertOnlyProtected(root, prefix) {
  for (const [node, name] of classTokens(root)) {
    assert.ok(
      name.startsWith(prefix),
      `${node.tagName.toLowerCase()} carries ${name}; a protected region allows only ${prefix}* hooks`,
    );
  }
}

/** Renders the install review through the real plan section. */
async function renderedPlan() {
  const document = installDom();
  installWindow({
    invoke: async (command) => (command === 'plan_install' ? REPORT : {}),
  });
  const section = createPlanSection({ gameId: 'game', modId: 'mod', path: 'C:/game' });
  document.body.append(section);
  section.open = true;
  fire(section, 'toggle');
  await flush();
  return section;
}

test('the install review marks its root and renders only review hooks', async () => {
  const section = await renderedPlan();
  assert.equal(section.getAttribute('data-modn-protected'), 'install-review');
  assert.equal(findAllByClass(section, 'modn-review__row').length, 2);
  assert.equal(findAllByClass(section, 'modn-review__blocker').length, 1);
  assert.equal(findAllByClass(section, 'modn-review__status--error').length, 1);
  assertOnlyProtected(section, 'modn-review');
});

test('the approval ask marks its root and renders only approval hooks', () => {
  const document = installDom();
  installWindow();
  const host = document.createElement('div');
  document.body.append(host);
  renderAsk(host, PERMISSIONS, () => {}, () => {});
  const ask = host.lastElementChild;
  assert.equal(ask.getAttribute('data-modn-protected'), 'approval');
  assert.equal(findAllByClass(ask, 'modn-approval__button').length, 2);
  assert.equal(findAllByClass(ask, 'modn-approval__path').length, 1);
  assertOnlyProtected(ask, 'modn-approval');
});
