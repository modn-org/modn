/**
 * A tiny DOM and window for the behaviour tests. It implements only the
 * members the desktop UI modules use.
 */

export class FakeNode {
  /** @param {string} tag */
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    /** @type {FakeNode[]} */
    this.children = [];
    /** @type {FakeNode|null} */
    this.parentNode = null;
    this.className = '';
    /** @type {Record<string, string>} */
    this.dataset = {};
    /** @type {Record<string, string>} */
    this.attributes = {};
    /** @type {Map<string, Function[]>} */
    this.listeners = new Map();
    this.hidden = false;
    this.disabled = false;
    this.type = '';
    this.title = '';
    this.value = '';
    this._text = '';
    this.style = { setProperty() {} };
  }

  get id() {
    return this.attributes.id || '';
  }

  set id(value) {
    this.setAttribute('id', value);
  }

  get textContent() {
    if (this.children.length === 0) {
      return this._text;
    }
    return this._text + this.children.map((child) => child.textContent).join('');
  }

  set textContent(value) {
    for (const child of this.children) {
      child.parentNode = null;
    }
    this.children = [];
    this._text = value === null || value === undefined ? '' : String(value);
  }

  get lastElementChild() {
    return this.children[this.children.length - 1] || null;
  }

  get childElementCount() {
    return this.children.length;
  }

  get isConnected() {
    let node = this;
    while (node.parentNode) {
      node = node.parentNode;
    }
    return globalThis.document && node === globalThis.document.body;
  }

  /** @param {...FakeNode} nodes */
  append(...nodes) {
    for (const node of nodes) {
      this.appendChild(node);
    }
  }

  /** @param {FakeNode} node */
  appendChild(node) {
    if (node.parentNode) {
      node.parentNode.children = node.parentNode.children.filter((child) => child !== node);
    }
    node.parentNode = this;
    this.children.push(node);
    return node;
  }

  /** @param {...FakeNode} nodes */
  replaceChildren(...nodes) {
    for (const child of this.children) {
      child.parentNode = null;
    }
    this.children = [];
    this._text = '';
    this.append(...nodes);
  }

  remove() {
    if (this.parentNode) {
      this.parentNode.children = this.parentNode.children.filter((child) => child !== this);
      this.parentNode = null;
    }
  }

  /** @param {FakeNode} node */
  contains(node) {
    let current = node;
    while (current) {
      if (current === this) {
        return true;
      }
      current = current.parentNode;
    }
    return false;
  }

  /** @param {string} name @param {unknown} value */
  setAttribute(name, value) {
    this.attributes[name] = String(value);
  }

  /** @param {string} name */
  getAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this.attributes, name)
      ? this.attributes[name]
      : null;
  }

  /** @param {string} name */
  removeAttribute(name) {
    delete this.attributes[name];
  }

  /** @param {string} type @param {Function} listener */
  addEventListener(type, listener) {
    const list = this.listeners.get(type) || [];
    list.push(listener);
    this.listeners.set(type, list);
  }

  /** @param {string} type @param {Function} listener */
  removeEventListener(type, listener) {
    const list = this.listeners.get(type) || [];
    this.listeners.set(type, list.filter((entry) => entry !== listener));
  }

  click() {
    if (this.disabled) {
      return;
    }
    const event = { type: 'click', target: this };
    for (const listener of [...(this.listeners.get('click') || [])]) {
      listener.call(this, event);
    }
  }

  /** @param {string} selector */
  querySelector(selector) {
    return this.querySelectorAll(selector)[0] || null;
  }

  /** @param {string} selector */
  querySelectorAll(selector) {
    const matches = matcher(selector);
    /** @type {FakeNode[]} */
    const found = [];
    const visit = (node) => {
      for (const child of node.children) {
        if (matches(child)) {
          found.push(child);
        }
        visit(child);
      }
    };
    visit(this);
    return found;
  }
}

/** @param {string} selector */
function matcher(selector) {
  if (selector.startsWith('.')) {
    const className = selector.slice(1);
    return (node) => node.className.split(/\s+/).includes(className);
  }
  if (selector.startsWith('#')) {
    const id = selector.slice(1);
    return (node) => node.id === id;
  }
  return () => false;
}

/** Installs a fresh document and returns it. */
export function installDom() {
  const body = new FakeNode('body');
  const document = {
    body,
    /** @param {string} tag */
    createElement(tag) {
      return new FakeNode(tag);
    },
    /** @param {string} id */
    getElementById(id) {
      const matches = (node) => node !== body && node.id === id;
      const visit = (node) => {
        for (const child of node.children) {
          if (matches(child)) {
            return child;
          }
          const deeper = visit(child);
          if (deeper) {
            return deeper;
          }
        }
        return null;
      };
      return visit(body);
    },
    addEventListener() {},
    removeEventListener() {},
  };
  globalThis.document = document;
  globalThis.Node = FakeNode;
  return document;
}

/**
 * Installs a window whose timers never fire on their own.
 * @param {{invoke?: Function, listen?: Function}} [options]
 */
export function installWindow({ invoke = async () => ({}), listen = async () => () => {} } = {}) {
  const timers = new Map();
  let nextTimer = 1;
  let nextRun = 1;
  const window = {
    __TAURI__: { core: { invoke }, event: { listen } },
    crypto: { randomUUID: () => `run-${nextRun++}` },
    setTimeout(callback) {
      const id = nextTimer++;
      timers.set(id, callback);
      return id;
    },
    clearTimeout(id) {
      timers.delete(id);
    },
  };
  globalThis.window = window;
  return { window, timers };
}

/** @param {string} reason */
export function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** Yields enough microtasks for the UI's async chains to settle. */
export async function flush(rounds = 20) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
  }
}

/** @param {FakeNode} root @param {(node: FakeNode) => boolean} predicate */
export function find(root, predicate) {
  if (predicate(root)) {
    return root;
  }
  for (const child of root.children) {
    const found = find(child, predicate);
    if (found) {
      return found;
    }
  }
  return null;
}

/** @param {FakeNode} root @param {string} testId */
export function findByTestId(root, testId) {
  return find(root, (node) => node.dataset && node.dataset.testid === testId);
}

/** @param {FakeNode} root @param {string} className */
export function findAllByClass(root, className) {
  return allByClass(root, className);
}

/** @param {FakeNode} root @param {string} className */
function allByClass(root, className) {
  const found = [];
  if (root.className.split(/\s+/).includes(className)) {
    found.push(root);
  }
  for (const child of root.children) {
    found.push(...allByClass(child, className));
  }
  return found;
}

/** Returns the toast host the app or showToast created. */
export function toastHost() {
  return globalThis.document.getElementById('modn-toasts');
}

/** Returns the newest toast, if any. */
export function lastToast() {
  const host = toastHost();
  return host ? host.lastElementChild : null;
}
