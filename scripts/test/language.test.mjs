import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {applyQueryPatch, checkRecipe, detectLicense, licenseExpression, rewriteCaptures} from '../lib/build-language.mjs';
import {iconSvg} from '../lib/documents.mjs';
import {checkout, cli, write} from './cli.mjs';
import {WASM} from './fixtures.mjs';

const recipeText = `schema = 1
kind = "language"

[extension]
name = "Demo"
version = "1.0.0"
description = "Syntax highlighting for Demo."

[sources.demo]
directory = "local/demo"
licenses = [ "LICENSE" ]

[grammars.demo]
source = "demo"
path = "."
prebuilt = "tree-sitter-demo.wasm"
file-types = [ "demo" ]
sample = "samples/demo.txt"

[[grammars.demo.highlights]]
path = "queries/highlights.scm"
`;

function demo(repo, text = recipeText) {
  const dir = path.join(repo.extensions, 'loom.lang.demo');
  write(dir, 'recipe.toml', text);
  write(dir, 'local/demo/tree-sitter-demo.wasm', WASM);
  write(dir, 'local/demo/queries/highlights.scm', '(identifier) @variable\n(comment) @note\n');
  write(dir, 'local/demo/LICENSE', 'MIT License\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software\n');
  write(dir, 'samples/demo.txt', 'hello');
  return dir;
}

test('captures are renamed outside comments and strings only', () => {
  const text = '(a) @note ; keep @note here\n(b) @other "@note"';
  assert.equal(rewriteCaptures(text, {note: 'comment'}), '(a) @comment ; keep @note here\n(b) @other "@note"');
  assert.equal(rewriteCaptures(text, {}), text);
});

test('common licenses are recognised and anything else is not asserted', () => {
  assert.equal(detectLicense('MIT License\nPermission is hereby granted, free of charge, to any person obtaining a copy'), 'MIT');
  assert.equal(detectLicense('Apache License\nVersion 2.0, January 2004'), 'Apache-2.0');
  assert.equal(detectLicense('Redistribution and use in source and binary forms, with or without modification'), 'BSD-2-Clause');
  assert.equal(detectLicense('Redistribution and use in source and binary forms ... Neither the name of the copyright holder'), 'BSD-3-Clause');
  assert.equal(detectLicense('GNU General Public License'), null);
  assert.equal(licenseExpression(['Apache License, Version 2.0', 'Permission is hereby granted, free of charge, to any person obtaining a copy']), 'Apache-2.0 AND MIT');
  assert.equal(licenseExpression(['all rights reserved']), 'NOASSERTION');
  assert.equal(licenseExpression([]), 'NOASSERTION');
});

test('icons are deterministic and escape the text', () => {
  assert.equal(iconSvg('loom.lang.c', 'C'), iconSvg('loom.lang.c', 'C'));
  assert.notEqual(iconSvg('loom.lang.c', 'C'), iconSvg('loom.lang.go', 'C'));
  assert.match(iconSvg('loom.lang.x', '<&'), />&lt;&amp;</);
});

test('a recipe is checked for pinning, sources, grammars and samples', t => {
  const repo = checkout(t);
  demo(repo);
  assert.deepEqual(checkRecipe('loom.lang.demo', {
    extension: {name: 'Demo', version: '1.0.0', description: 'Demo.'},
    sources: {demo: {directory: 'local/demo', licenses: ['LICENSE']}},
    grammars: {demo: {source: 'demo', path: '.', prebuilt: 'tree-sitter-demo.wasm', 'file-types': ['demo'], sample: 'samples/demo.txt', highlights: [{path: 'queries/highlights.scm'}]}},
  }, {root: repo.extensions}), []);

  const problems = checkRecipe('loom.lang.demo', {
    extension: {name: '', version: '1', description: 'x', recommends: ['loom.theme.x']},
    sources: {
      remote: {repository: 'https://example.com/x', revision: 'abc', sha256: 'zz', licenses: []},
      Bad: {directory: 'local/ghost', licenses: ['LICENSE']},
    },
    grammars: {demo: {source: 'ghost', 'file-types': [''], highlights: [], sample: 'samples/missing.txt', generate: true, prebuilt: 'x.wasm'}},
    patches: ['patches/none.patch'],
  }, {root: repo.extensions}).join('\n');
  for (const expected of [/extension\.name is required/, /extension\.version must be/, /recommends: loom\.theme\.x/, /expected a GitHub repository URL/,
    /pin a complete commit SHA/, /pin the archive SHA-256/, /list its license files/, /invalid name/, /unknown source ghost/, /file-types must list/,
    /missing highlights/, /missing sample samples\/missing\.txt/, /cannot generate and use a prebuilt/, /missing patch/]) {
    assert.match(problems, expected);
  }
});

test('a patch may only change package queries', t => {
  const repo = checkout(t);
  const stage = path.join(repo.root, 'stage');
  write(stage, 'manifest.json', '{}');
  const patch = write(repo.root, 'evil.patch', '--- a/manifest.json\n+++ b/manifest.json\n@@ -1 +1 @@\n-{}\n+{"x":1}\n');
  assert.throws(() => applyQueryPatch(stage, patch), /only modify package queries/);
});

test('a language builds from a recipe with a prebuilt module and no network', t => {
  const repo = checkout(t, {bundled: []});
  demo(repo);
  const check = cli(repo.root, ['check']);
  assert.equal(check.status, 0, check.all);
  const built = cli(repo.root, ['build', 'loom.lang.demo', '--offline']);
  assert.equal(built.status, 0, built.all);

  const stage = path.join(repo.root, 'dist/stage/any/loom.lang.demo');
  const manifest = JSON.parse(fs.readFileSync(path.join(stage, 'manifest.json'), 'utf8'));
  assert.equal(manifest.id, 'loom.lang.demo');
  assert.equal(manifest.name, 'Demo Language');
  assert.equal(manifest.license, 'MIT');
  assert.deepEqual(manifest.contributes.languages, [{id: 'loom.lang.demo.grammar', path: 'tree-sitter.json'}]);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(stage, 'tree-sitter.json'), 'utf8')),
    {grammars: [{name: 'demo', path: '.', 'file-types': ['demo'], highlights: 'queries/demo/highlights.scm'}]});
  for (const file of ['tree-sitter-demo.wasm', 'queries/demo/highlights.scm', 'README.md', 'CHANGELOG.md', 'icon.svg', 'PROVENANCE.md', 'licenses/demo-0-LICENSE']) {
    assert.ok(fs.existsSync(path.join(stage, file)), file);
  }
  const packed = cli(repo.root, ['pack', 'loom.lang.demo']);
  assert.equal(packed.status, 0, packed.all);
  assert.ok(fs.existsSync(path.join(repo.root, 'dist/packages/loom.lang.demo-1.0.0-any.loomext')));
});

test('a grammar that must be compiled asks for the toolchain', t => {
  const repo = checkout(t, {bundled: []});
  demo(repo, recipeText.replace('prebuilt = "tree-sitter-demo.wasm"', 'generate = true'));
  write(path.join(repo.extensions, 'loom.lang.demo'), 'local/demo/src/parser.c', '');
  const built = cli(repo.root, ['build', 'loom.lang.demo', '--offline']);
  assert.equal(built.status, 1);
  assert.match(built.err, /tools\/toolchain\.toml is needed/);
});
