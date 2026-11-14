import assert from 'node:assert/strict';
import test from 'node:test';

import { mergeLibrary, visibleModels } from '../../ui/js/library/data.js';

const ENTRY = {
  id: 'cool-mod',
  bucket: 'main',
  name: 'Cool Mod',
  version: '2.0.0',
};

test('keeps two installed packages that share one mod ID', () => {
  const packages = [
    {
      id: 'pkg-main',
      version: '1.0.0',
      catalog: { bucket: 'main', game_id: 'game', mod_id: 'cool-mod' },
    },
    {
      id: 'pkg-other',
      version: '1.5.0',
      catalog: { bucket: 'other', game_id: 'game', mod_id: 'cool-mod' },
    },
  ];
  const models = mergeLibrary([ENTRY], packages);
  assert.equal(models.length, 2, 'both packages must stay in the grid');
  assert.deepEqual(
    models.map((model) => model.installedPackage.id).sort(),
    ['pkg-main', 'pkg-other'],
  );
  assert.equal(new Set(models.map((model) => model.key)).size, 2, 'tile keys must differ');
  const catalogRow = models.find((model) => model.installedPackage.id === 'pkg-main');
  assert.equal(catalogRow.key, 'catalog-main-cool-mod-pkg-main');
  assert.equal(catalogRow.hasUpdate, true, 'the catalog row compares its own package version');
  const extra = models.find((model) => model.installedPackage.id === 'pkg-other');
  assert.equal(extra.key, 'package-pkg-other');
  assert.equal(extra.bucket, 'other');
});

test('matches one package to each catalog entry of the same mod ID', () => {
  const entries = [
    { id: 'cool-mod', bucket: 'main', name: 'Cool Mod', version: '2.0.0' },
    { id: 'cool-mod', bucket: 'other', name: 'Cool Mod', version: '3.0.0' },
  ];
  const packages = [
    {
      id: 'pkg-main',
      version: '2.0.0',
      catalog: { bucket: 'main', game_id: 'game', mod_id: 'cool-mod' },
    },
    {
      id: 'pkg-other',
      version: '1.0.0',
      catalog: { bucket: 'other', game_id: 'game', mod_id: 'cool-mod' },
    },
  ];
  const models = mergeLibrary(entries, packages);
  assert.equal(models.length, 2, 'each catalog entry renders its own row');
  const main = models.find((model) => model.bucket === 'main');
  const other = models.find((model) => model.bucket === 'other');
  assert.equal(main.installedPackage.id, 'pkg-main');
  assert.equal(main.hasUpdate, false);
  assert.equal(other.installedPackage.id, 'pkg-other');
  assert.equal(other.hasUpdate, true);
});

test('keeps unmatched packages and manual packages beside catalog rows', () => {
  const packages = [
    {
      id: 'pkg-catalog',
      version: '1.0.0',
      catalog: { bucket: 'main', game_id: 'game', mod_id: 'cat-mod' },
    },
    { id: 'pkg-archive', mod_id: 'archive-mod', version: '4.2.0' },
    { id: 'manual-1', version: '9.0.0', catalog: null },
  ];
  const models = mergeLibrary([ENTRY], packages);
  assert.deepEqual(
    models.map((model) => model.key).sort(),
    ['catalog-main-cool-mod', 'manual-manual-1', 'package-pkg-archive', 'package-pkg-catalog'],
  );
  const archive = models.find((model) => model.key === 'package-pkg-archive');
  assert.equal(archive.modId, 'archive-mod');
  assert.equal(archive.hasUpdate, false, 'an unmatched package never offers Update');
  assert.equal(archive.installedPackage.id, 'pkg-archive');
  const manual = models.find((model) => model.key === 'manual-manual-1');
  assert.equal(manual.modId, 'manual-1');
});

test('binds a catalog package only once', () => {
  const pkg = {
    id: 'pkg-main',
    version: '1.0.0',
    catalog: { bucket: 'main', game_id: 'game', mod_id: 'cool-mod' },
  };
  const models = mergeLibrary([ENTRY], [pkg]);
  assert.equal(models.length, 1);
  assert.equal(models[0].key, 'catalog-main-cool-mod-pkg-main');
  assert.equal(models[0].installedPackage, pkg);
});

test('installed filter keeps every package of a shared mod ID', () => {
  const packages = [
    { id: 'pkg-a', catalog: { bucket: 'main', mod_id: 'cool-mod' } },
    { id: 'pkg-b', catalog: { bucket: 'other', mod_id: 'cool-mod' } },
  ];
  const models = mergeLibrary([ENTRY], packages);
  assert.equal(visibleModels(models, 'installed', '').length, 2);
  assert.equal(visibleModels(models, 'available', '').length, 0);
});
