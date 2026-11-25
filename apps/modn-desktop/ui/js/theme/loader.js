/**
 * Applies a resolved theme to the document with constructable stylesheets.
 *
 * The loader is pure apart from the sheets it owns: the caller injects the
 * document and the sheet factory, so the behaviour tests need no CSS engine.
 * @typedef {Object} ThemeSource
 * @property {string} kind
 * @property {string|null} bucket
 * @property {string} label
 * @typedef {Object} ResolvedTheme
 * @property {string} [game]
 * @property {{id: string, name: string, source: ThemeSource,
 *   variant: string|null, css: string, variant_css: string|null}|null} theme
 * @property {{reason: string, message: string}|null} [fallback]
 * @property {Object[]} [diagnostics]
 * @property {string} [asset_base]
 * @property {string} [disabled]
 */
const WINDOWS_ASSET_BASE = `http${'://'}modn-theme.localhost/`;
const NATIVE_ASSET_BASE = 'modn-theme://localhost/';
const ALLOWED_ASSET_BASES = [WINDOWS_ASSET_BASE, NATIVE_ASSET_BASE];
let ownedSheets = [];
/**
 * Prefixes each theme-relative asset URL with the protocol base.
 *
 * Only the three canonical forms `url(assets/`, `url("assets/` and
 * `url('assets/` are rewritten. The quote style is kept.
 * @param {string} css
 * @param {string} assetBase
 * @returns {string}
 */
export function rewriteAssetUrls(css, assetBase) {
  if (!ALLOWED_ASSET_BASES.includes(assetBase)) {
    throw new Error('unexpected theme asset base');
  }
  if (typeof css !== 'string') {
    throw new Error('theme css must be text');
  }
  return css.replace(/url\((["']?)assets\//g, (_match, quote) => {
    return `url(${quote}${assetBase}assets/`;
  });
}
function layerThemeCss(css, assetBase) {
  return `@layer theme {\n${rewriteAssetUrls(css, assetBase)}\n}`;
}
function adopted(document) {
  return Array.from(document.adoptedStyleSheets || []);
}
function adopt(document, sheets) {
  document.adoptedStyleSheets = sheets;
}
function releaseThemeSheets(document) {
  if (ownedSheets.length === 0) {
    return;
  }
  const kept = adopted(document).filter((sheet) => !ownedSheets.includes(sheet));
  ownedSheets = [];
  adopt(document, kept);
}
/**
 * Applies the resolved theme and its selected variant sheet.
 *
 * The theme sheet is added first and the variant second, after every sheet
 * that already belongs to the document. A failure removes both the theme sheet
 * and the variant sheet, so the default theme shows.
 * @param {ResolvedTheme} resolved
 * @param {{document: Document, createSheet: () => {replaceSync: Function}}} env
 * @returns {{applied: boolean, name?: string, source?: ThemeSource,
 *   variant?: string|null, error?: Error|null}}
 */
export function applyTheme(resolved, env) {
  const { document, createSheet } = env || {};
  if (!document || typeof createSheet !== 'function') {
    return { applied: false, error: new Error('the theme environment is incomplete') };
  }
  releaseThemeSheets(document);
  const theme = resolved && resolved.theme ? resolved.theme : null;
  if (theme === null || (resolved && resolved.disabled)) {
    return { applied: false, error: null };
  }
  try {
    const themeSheet = createSheet();
    themeSheet.replaceSync(layerThemeCss(theme.css, resolved.asset_base));
    adopt(document, [...adopted(document), themeSheet]);
    ownedSheets.push(themeSheet);
    if (theme.variant_css !== null && theme.variant_css !== undefined) {
      const variantSheet = createSheet();
      variantSheet.replaceSync(layerThemeCss(theme.variant_css, resolved.asset_base));
      adopt(document, [...adopted(document), variantSheet]);
      ownedSheets.push(variantSheet);
    }
    return {
      applied: true,
      name: theme.name,
      source: theme.source,
      variant: theme.variant === undefined ? null : theme.variant,
    };
  } catch (error) {
    releaseThemeSheets(document);
    return { applied: false, error };
  }
}
/**
 * Returns the production environment. A webview without constructable
 * stylesheets makes `createSheet` throw, which the loader reports as a load
 * error and turns into the default theme.
 * @returns {{document: Document, createSheet: () => CSSStyleSheet}}
 */
export function browserEnvironment() {
  return {
    document: globalThis.document,
    createSheet: () => {
      if (typeof CSSStyleSheet !== 'function') {
        throw new Error('CSSStyleSheet is unavailable');
      }
      return new CSSStyleSheet();
    },
  };
}
