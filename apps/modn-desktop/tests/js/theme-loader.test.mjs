import assert from 'node:assert/strict';
import test from 'node:test';
import {
  applyTheme,
  browserEnvironment,
  rewriteAssetUrls,
} from '../../ui/js/theme/loader.js';

const BASE = 'http://modn-theme.localhost/';

/** A document with an adopted sheet list. */
function documentWith(sheets = []) {
  return { adoptedStyleSheets: [...sheets] };
}

/** A sheet factory that records every replacement and can fail on demand. */
function sheetFactory({ failOn = 0 } = {}) {
  let made = 0;
  return () => {
    made += 1;
    const number = made;
    return {
      text: null,
      replaceSync(value) {
        if (number === failOn) {
          throw new Error('the stylesheet cannot be parsed');
        }
        this.text = value;
      },
    };
  };
}

/** One resolve answer with a theme and a variant. */
function resolved(overrides = {}) {
  return {
    game: 'fixture',
    asset_base: BASE,
    theme: {
      id: 'night-city',
      name: 'Night City',
      source: { kind: 'official', bucket: 'main', label: 'main bucket' },
      variant: 'neon',
      css: '.modn-app{background-image:url(assets/bg.webp)}',
      variant_css: '.modn-app{color:#ffffff}',
    },
    fallback: null,
    diagnostics: [],
    ...overrides,
  };
}

test('rewrites the unquoted asset url form', () => {
  assert.equal(
    rewriteAssetUrls('a{background:url(assets/bg.webp)}', BASE),
    `a{background:url(${BASE}assets/bg.webp)}`,
  );
});

test('rewrites the double-quoted asset url form', () => {
  assert.equal(
    rewriteAssetUrls('@font-face{src:url("assets/f.woff2")}', BASE),
    `@font-face{src:url("${BASE}assets/f.woff2")}`,
  );
});

test('rewrites the single-quoted asset url form', () => {
  assert.equal(
    rewriteAssetUrls('a{background:url(\'assets/i.png\')}', BASE),
    `a{background:url('${BASE}assets/i.png')}`,
  );
});

test('keeps every other url untouched', () => {
  const css = 'a{background:url(../assets/x.png)}b{background:url(data:x)}c{x:url(assetsx/y)}';
  assert.equal(rewriteAssetUrls(css, BASE), css);
});

test('accepts the native scheme base', () => {
  assert.equal(
    rewriteAssetUrls('url(assets/a.png)', 'modn-theme://localhost/'),
    'url(modn-theme://localhost/assets/a.png)',
  );
});

test('refuses a base without the trailing slash', () => {
  assert.throws(() => rewriteAssetUrls('', 'http://modn-theme.localhost'));
});

test('refuses another origin', () => {
  assert.throws(() => rewriteAssetUrls('', 'https://example.test/'));
});

test('refuses a missing base', () => {
  assert.throws(() => rewriteAssetUrls('url(assets/a.png)', undefined));
});

test('adds the theme sheet then the variant sheet after the existing sheets', () => {
  const existing = { name: 'app' };
  const document = documentWith([existing]);
  const createSheet = sheetFactory();
  const result = applyTheme(resolved(), { document, createSheet });
  assert.equal(result.applied, true);
  assert.equal(result.name, 'Night City');
  assert.equal(result.variant, 'neon');
  assert.equal(result.source.label, 'main bucket');
  assert.equal(document.adoptedStyleSheets.length, 3);
  assert.equal(document.adoptedStyleSheets[0], existing);
  assert.equal(
    document.adoptedStyleSheets[1].text,
    `.modn-app{background-image:url(${BASE}assets/bg.webp)}`,
  );
  assert.equal(document.adoptedStyleSheets[2].text, '.modn-app{color:#ffffff}');
});

test('keeps every adopted sheet that the theme did not add', () => {
  const first = { name: 'first' };
  const second = { name: 'second' };
  const document = documentWith([first, second]);
  applyTheme(resolved({ theme: null }), { document, createSheet: sheetFactory() });
  assert.deepEqual(document.adoptedStyleSheets, [first, second]);
});

test('a second apply removes the sheets of the first', () => {
  const document = documentWith();
  applyTheme(resolved(), { document, createSheet: sheetFactory() });
  const oldSheets = [...document.adoptedStyleSheets];
  applyTheme(resolved({ theme: { ...resolved().theme, variant_css: null } }), {
    document,
    createSheet: sheetFactory(),
  });
  assert.equal(document.adoptedStyleSheets.length, 1);
  assert.ok(!oldSheets.includes(document.adoptedStyleSheets[0]));
});

test('rolls the theme sheet back when the variant sheet throws', () => {
  const existing = { name: 'app' };
  const document = documentWith([existing]);
  const result = applyTheme(resolved(), { document, createSheet: sheetFactory({ failOn: 2 }) });
  assert.equal(result.applied, false);
  assert.match(result.error.message, /cannot be parsed/);
  assert.deepEqual(document.adoptedStyleSheets, [existing]);
});

test('a null theme removes earlier sheets and reports no error', () => {
  const document = documentWith();
  applyTheme(resolved(), { document, createSheet: sheetFactory() });
  const result = applyTheme({ theme: null }, { document, createSheet: sheetFactory() });
  assert.deepEqual(result, { applied: false, error: null });
  assert.equal(document.adoptedStyleSheets.length, 0);
});

test('the default theme flag removes earlier sheets', () => {
  const document = documentWith();
  applyTheme(resolved(), { document, createSheet: sheetFactory() });
  const result = applyTheme(
    { theme: null, disabled: 'default-theme-flag' },
    { document, createSheet: sheetFactory() },
  );
  assert.deepEqual(result, { applied: false, error: null });
  assert.equal(document.adoptedStyleSheets.length, 0);
});

test('a webview without a sheet factory falls back with an error', () => {
  const document = documentWith();
  const result = applyTheme(resolved(), { document });
  assert.equal(result.applied, false);
  assert.match(result.error.message, /environment/);
  assert.equal(document.adoptedStyleSheets.length, 0);
});

test('a webview without CSSStyleSheet reports the load error', () => {
  const document = documentWith();
  const result = applyTheme(resolved(), {
    document,
    createSheet: () => {
      throw new Error('CSSStyleSheet is unavailable');
    },
  });
  assert.equal(result.applied, false);
  assert.match(result.error.message, /CSSStyleSheet/);
  assert.equal(document.adoptedStyleSheets.length, 0);
});

test('the browser environment throws when CSSStyleSheet is missing', () => {
  const env = browserEnvironment();
  assert.throws(() => env.createSheet(), /CSSStyleSheet/);
});
