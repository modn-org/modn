/**
 * Builds the one-time approval ask. Every permission and every executable path
 * is its own text node: refusal data is untrusted and never becomes markup.
 */

/**
 * @param {string} className
 * @param {string} text
 * @returns {HTMLElement}
 */
function line(className, text) {
  const node = document.createElement('p');
  node.className = className;
  node.textContent = text;
  return node;
}

/**
 * @param {HTMLElement} host
 * @param {{native: boolean, executables: string[]}} permissions
 * @param {() => void} onAllow
 * @param {() => void} onDecline
 */
export function renderAsk(host, permissions, onAllow, onDecline) {
  host.replaceChildren();
  const ask = document.createElement('div');
  ask.className = 'modn-ask';
  ask.setAttribute('role', 'group');
  ask.setAttribute('aria-label', 'Approval request');
  ask.append(line('modn-ask__question', 'This change needs your approval.'));
  if (permissions.native) {
    ask.append(line('modn-ask__question', 'Allow native code through the native injector.'));
  }
  if (permissions.executables.length > 0) {
    ask.append(line('modn-ask__question', 'Allow these executables:'));
    const list = document.createElement('ul');
    list.className = 'modn-ask__list';
    for (const path of permissions.executables) {
      const item = document.createElement('li');
      item.className = 'modn-ask__path';
      item.textContent = path;
      list.append(item);
    }
    ask.append(list);
  }
  const actions = document.createElement('div');
  actions.className = 'modn-ask__actions';
  const allow = document.createElement('button');
  allow.type = 'button';
  allow.className = 'modn-button';
  allow.textContent = 'Allow';
  allow.dataset.testid = 'ask-allow';
  const decline = document.createElement('button');
  decline.type = 'button';
  decline.className = 'modn-button modn-button--quiet';
  decline.textContent = 'Decline';
  decline.dataset.testid = 'ask-decline';
  allow.addEventListener('click', () => {
    host.replaceChildren();
    onAllow();
  });
  decline.addEventListener('click', () => {
    host.replaceChildren();
    onDecline();
  });
  actions.append(allow, decline);
  ask.append(actions);
  host.append(ask);
}
