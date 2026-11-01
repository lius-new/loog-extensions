// Builds small extension folders in a temporary directory for the tests.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export function workspace(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'loom-ext-test-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  return root;
}

export function write(root, file, content) {
  const target = path.join(root, ...file.split('/'));
  fs.mkdirSync(path.dirname(target), {recursive: true});
  fs.writeFileSync(target, typeof content === 'string' || Buffer.isBuffer(content) ? content : JSON.stringify(content, null, 2));
  return target;
}

export const WASM = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0, 0, 0]);

const base = (id, extra = {}) => ({
  schema: 1, id, version: '1.0.0', name: 'Test', publisher: 'Loom', description: 'A test.',
  categories: ['Syntax Highlighting'], license: 'MIT', repository: 'https://github.com/acme/extensions',
  icon: 'icon.svg', readme: 'README.md', changelog: 'CHANGELOG.md',
  engines: {loom: '>=0.1.8 <0.3'}, ...extra,
});

/** The files every published extension carries beside its manifest. */
function documents(dir) {
  write(dir, 'icon.svg', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8"/></svg>');
  write(dir, 'README.md', '# Test\n');
  write(dir, 'CHANGELOG.md', '# Changelog\n');
}

/** A folder `extensions/<id>` holding a valid theme extension. */
export function theme(root, id = 'loom.theme.demo') {
  const dir = path.join(root, id);
  write(dir, 'manifest.json', base(id, {categories: ['Themes'], contributes: {themes: [{id: `${id}.dark`, path: 'themes/dark.json'}]}}));
  write(dir, 'themes/dark.json', {name: 'Dark', base: 'aura-dark'});
  documents(dir);
  return dir;
}

/** A valid language extension with one grammar. */
export function language(root, id = 'loom.lang.demo', name = 'demo') {
  const dir = path.join(root, id);
  write(dir, 'manifest.json', base(id, {contributes: {highlight: [{id: `${id}.grammar`, path: 'tree-sitter.json'}]}}));
  write(dir, 'tree-sitter.json', {grammars: [{name, 'file-types': [name]}]});
  write(dir, `tree-sitter-${name}.wasm`, WASM);
  write(dir, 'queries/highlights.scm', '(identifier) @variable');
  documents(dir);
  return dir;
}

/** A valid language server extension whose binary is in the folder. */
export function server(root, id = 'loom.lsp.demo') {
  const dir = path.join(root, id);
  write(dir, 'manifest.json', base(id, {
    categories: ['Language Services'],
    capabilities: ['process.execute-local'],
    contributes: {language_service: {servers: [{id: `${id}.server`, languages: ['demo'], command: '${extension}/bin/demo-ls', root_markers: ['demo.toml']}]}},
  }));
  write(dir, 'bin/demo-ls', 'binary');
  documents(dir);
  return dir;
}
