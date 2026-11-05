import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {checkExtension, parseRequirement, validId} from '../lib/manifest.mjs';
import {language, server, theme, workspace, write} from './fixtures.mjs';

const problems = (dir, options) => checkExtension(dir, options).problems;

test('ids need two or more lowercase segments', () => {
  for (const id of ['loom.lang.rust', 'a.b', 'loom.lang.c-sharp']) assert.ok(validId(id), id);
  for (const id of ['loom', 'Loom.lang', 'loom..x', 'loom.-x', '', 'a.b_c']) assert.ok(!validId(id), id);
});

test('requirements are an id and a range', () => {
  assert.deepEqual(parseRequirement('loom.runtime.node@^22'), {id: 'loom.runtime.node', range: '^22'});
  for (const bad of ['loom.runtime.node', '@^22', 'node@^22', 'loom.runtime.node@>=a.b']) assert.equal(parseRequirement(bad), null, bad);
});

test('valid theme, language and server extensions have no problems', t => {
  const root = workspace(t);
  assert.deepEqual(problems(theme(root)), []);
  assert.deepEqual(problems(language(root)), []);
  assert.deepEqual(problems(server(root), {built: true}), []);
});

test('language service identities work without grammars and reject parser bindings', t => {
  const dir = server(workspace(t));
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  manifest.contributes.languages = [{ id: `${manifest.id}.identity`, path: 'languages/demo/language.json' }];
  write(dir, 'manifest.json', manifest);
  const identity = { name: 'demo', variants: [{ id: 'demo' }], files: [{ extensions: ['demo'], variant: 'demo' }] };
  write(dir, 'languages/demo/language.json', identity);
  assert.deepEqual(problems(dir), []);
  identity.variants[0].grammar = 'demo';
  write(dir, 'languages/demo/language.json', identity);
  assert.match(problems(dir).join('\n'), /without parser bindings/);
});

test('language categories describe a specific function and reject Languages', t => {
  const root = workspace(t);
  for (const [dir, category] of [[language(root), 'Syntax Highlighting'], [server(root), 'Language Services']]) {
    const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
    assert.deepEqual(manifest.categories, [category]);
    assert.deepEqual(problems(dir, {built: true}), []);
    manifest.categories = ['Languages'];
    write(dir, 'manifest.json', manifest);
    assert.match(problems(dir).join('\n'), /"Languages" is not a category/);
  }
});

test('the id must match the folder and use the naming scheme', t => {
  const root = workspace(t);
  const dir = theme(root);
  write(dir, 'manifest.json', {...JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8')), id: 'loom.theme.other'});
  assert.match(problems(dir).join('\n'), /folder is named loom\.theme\.demo but the id is loom\.theme\.other/);
  const lang = language(root);
  write(lang, 'manifest.json', {schema: 1, id: 'loom.lang.demo', version: '1.0.0', name: 'x', publisher: 'x', description: 'x', categories: ['x'], license: 'MIT', engines: {loom: '*'}});
  assert.match(problems(lang).join('\n'), /must contribute highlight/);
});

test('publishing needs what people see; development does not', t => {
  const root = workspace(t);
  const dir = theme(root);
  write(dir, 'manifest.json', {schema: 1, id: 'loom.theme.demo', version: '1.0.0', contributes: {themes: [{id: 'loom.theme.demo.dark', path: 'themes/dark.json'}]}});
  const published = problems(dir).join('\n');
  for (const field of ['name', 'publisher', 'description', 'license', 'categories', 'engines.loom']) assert.match(published, new RegExp(field));
  assert.deepEqual(problems(dir, {publishing: false}), []);
});

test('resource paths stay inside, exist, and are namespaced', t => {
  const root = workspace(t);
  const dir = theme(root);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  manifest.contributes.themes = [{id: 'other.dark', path: 'themes/dark.json'}, {id: 'loom.theme.demo.x', path: '../escape.json'}, {id: 'loom.theme.demo.y', path: 'themes/missing.json'}];
  write(dir, 'manifest.json', manifest);
  const text = problems(dir).join('\n');
  assert.match(text, /must start with loom\.theme\.demo\./);
  assert.match(text, /Invalid theme path/);
  assert.match(text, /themes\/missing\.json does not exist/);
});

test('a grammar must come with a real WebAssembly module and its queries', t => {
  const root = workspace(t);
  const dir = language(root);
  fs.rmSync(path.join(dir, 'tree-sitter-demo.wasm'));
  assert.match(problems(dir).join('\n'), /missing WebAssembly module for demo/);
  write(dir, 'tree-sitter-demo.wasm', 'not wasm at all');
  assert.match(problems(dir).join('\n'), /is not a WebAssembly module/);
  const other = language(root, 'loom.lang.other', 'other');
  fs.rmSync(path.join(other, 'queries/highlights.scm'));
  assert.match(problems(other).join('\n'), /missing queries\/highlights\.scm/);
});

test('contribution points are named by what they are for, and an old or unknown name is refused', t => {
  const root = workspace(t);
  const dir = language(root);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  // Syntax grammars cannot be passed off as metadata-only identities.
  manifest.contributes = {languages: manifest.contributes.highlight};
  write(dir, 'manifest.json', manifest);
  const text = problems(dir).join('\n');
  assert.match(text, /language identity .*must name language\.json/);
  assert.match(text, /must contribute highlight/);
  manifest.contributes = {highlight: [{id: 'loom.lang.demo.grammar', path: 'tree-sitter.json'}], language_service: {servers: []}, themes: []};
  write(dir, 'manifest.json', manifest);
  assert.deepEqual(problems(dir), []);
  manifest.contributes.language_service = {language_servers: []};
  write(dir, 'manifest.json', manifest);
  assert.match(problems(dir).join('\n'), /contributes\.language_service\.language_servers is not part of the language service/);
});

test('a language service name is one short line', t => {
  const root = workspace(t);
  const dir = server(root);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  manifest.contributes.language_service.name = 'Demo Language Service';
  write(dir, 'manifest.json', manifest);
  assert.deepEqual(problems(dir, {built: true}), []);
  for (const bad of ['', '  ', 'two\nlines', 'x'.repeat(41), 7]) {
    manifest.contributes.language_service.name = bad;
    write(dir, 'manifest.json', manifest);
    assert.match(problems(dir, {built: true}).join('\n'), /language_service\.name must be one line/, JSON.stringify(bad));
  }
});

test('native libraries are refused in highlight and theme extensions but allowed in language servers', t => {
  const root = workspace(t);
  const dir = language(root);
  write(dir, 'rust.dll', 'native');
  assert.match(problems(dir).join('\n'), /native libraries are not allowed/);
  const lsp = server(root);
  write(lsp, 'bin/helper.dll', 'native');
  assert.deepEqual(problems(lsp, {built: true}), []);
});

test('a language server declares execution, a placeholder command and root markers', t => {
  const root = workspace(t);
  const dir = server(root);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  manifest.capabilities = [];
  manifest.contributes.language_service.servers[0].command = 'demo-ls';
  manifest.contributes.language_service.servers[0].root_markers = [];
  write(dir, 'manifest.json', manifest);
  const text = problems(dir, {built: true}).join('\n');
  assert.match(text, /declare process\.execute-local/);
  assert.match(text, /command must start with/);
  assert.match(text, /root_markers is required/);
  manifest.capabilities = ['process.execute-local'];
  manifest.contributes.language_service.servers[0].command = '${extension}/bin/absent';
  manifest.contributes.language_service.servers[0].root_markers = ['x'];
  write(dir, 'manifest.json', manifest);
  assert.match(problems(dir, {built: true}).join('\n'), /bin\/absent is not in the package/);
  assert.deepEqual(problems(dir, {built: false}), []);
});

test('requires, recommends and capabilities are checked', t => {
  const root = workspace(t);
  const dir = language(root);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  Object.assign(manifest, {requires: ['bad'], recommends: ['loom.theme.x'], capabilities: ['teleport']});
  write(dir, 'manifest.json', manifest);
  const text = problems(dir).join('\n');
  assert.match(text, /invalid requirement/);
  assert.match(text, /recommends only language service extensions/);
  assert.match(text, /unknown capability teleport/);
});
