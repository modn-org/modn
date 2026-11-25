export class FakeNode {
  constructor(tag) {
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.parentNode = null;
    this.className = '';
    this.dataset = {};
    this.attributes = {};
    this.listeners = new Map();
    this.hidden = false;
    this.disabled = false;
    this.type = '';
    this.title = '';
    this.value = '';
    this._text = '';
    this.style = { setProperty() {} };
    const tokenList = () => this.className.split(/\s+/).filter(Boolean);
    this.classList = {
      add: (...names) => {
        const tokens = tokenList();
        for (const name of names) {
          if (!tokens.includes(name)) {
            tokens.push(name);
          }
        }
        this.className = tokens.join(' ');
      },
      remove: (...names) => {
        this.className = tokenList()
          .filter((token) => !names.includes(token))
          .join(' ');
      },
      toggle: (name, force) => {
        const present = tokenList().includes(name);
        const wanted = force === undefined ? !present : force;
        if (wanted) {
          this.classList.add(name);
        } else {
          this.classList.remove(name);
        }
        return wanted;
      },
      contains: (name) => tokenList().includes(name),
    };
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

  append(...nodes) {
    for (const node of nodes) {
      this.appendChild(node);
    }
  }

  appendChild(node) {
    if (node.parentNode) {
      node.parentNode.children = node.parentNode.children.filter((child) => child !== node);
    }
    node.parentNode = this;
    this.children.push(node);
    return node;
  }

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

  setAttribute(name, value) {
    this.attributes[name] = String(value);
  }

  getAttribute(name) {
    return Object.prototype.hasOwnProperty.call(this.attributes, name)
      ? this.attributes[name]
      : null;
  }

  removeAttribute(name) {
    delete this.attributes[name];
  }

  addEventListener(type, listener) {
    const list = this.listeners.get(type) || [];
    list.push(listener);
    this.listeners.set(type, list);
  }

  removeEventListener(type, listener) {
    const list = this.listeners.get(type) || [];
    this.listeners.set(type, list.filter((entry) => entry !== listener));
  }

  focus() {
    if (globalThis.document) {
      globalThis.document.activeElement = this;
    }
  }

  blur() {
    if (globalThis.document && globalThis.document.activeElement === this) {
      globalThis.document.activeElement = globalThis.document.body;
    }
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

  querySelector(selector) {
    return this.querySelectorAll(selector)[0] || null;
  }

  querySelectorAll(selector) {
    const matches = matcher(selector);
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

export function installDom() {
  const body = new FakeNode('body');
  const document = {
    body,
    activeElement: body,
    createElement(tag) {
      return new FakeNode(tag);
    },
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

export function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

export async function flush(rounds = 20) {
  for (let index = 0; index < rounds; index += 1) {
    await Promise.resolve();
  }
}

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

export function findByTestId(root, testId) {
  return find(root, (node) => node.dataset && node.dataset.testid === testId);
}

export function findAllByClass(root, className) {
  return allByClass(root, className);
}

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

export function toastHost() {
  return globalThis.document.getElementById('modn-toasts');
}

export function lastToast() {
  const host = toastHost();
  return host ? host.lastElementChild : null;
}
