// Checks an extension folder with the rules Loom applies when it loads one, and
// the stricter ones a package must meet to be published. Problems are collected,
// not thrown, so one run reports everything wrong.
import fs from 'node:fs';
import path from 'node:path';
import {files, readJson, relative} from './util.mjs';
import {validRange, validVersion} from './semver.mjs';

// Keep in step with `CAPABILITIES` and `EVENTS` in Loom's src/features/extensions/manifest.rs.
export const CAPABILITIES = ['documents.read', 'workspace.edit', 'workspace.files.read', 'workspace.files.write',
  'events', 'settings', 'languages', 'storage', 'network', 'tasks', 'process.execute-local'];
export const EVENTS = ['document.open', 'document.change', 'document.close', 'document.save',
  'selection.change', 'focus.change', 'root.change'];

const SEGMENT = /^[a-z0-9][a-z0-9-]*$/;
export const validId = id => typeof id === 'string' && id.split('.').length >= 2 && id.split('.').every(part => SEGMENT.test(part));
const GRAMMAR = /^[a-z][a-z0-9_]*$/;
const NATIVE = /\.(dll|so|dylib)$/i;
const QUERY_FILES = ['highlights.scm', 'injections.scm'];

/** What a package of this id must contribute, by the naming scheme. */
export const KINDS = [
  {prefix: 'loom.lang.', name: 'language', need: c => c.languages?.length, message: 'contribute a language'},
  {prefix: 'loom.lsp.', name: 'language server', need: c => c.language_servers?.length, message: 'declare a language server'},
  {prefix: 'loom.theme.', name: 'theme', need: c => c.themes?.length, message: 'contribute a theme'},
  {prefix: 'loom.queries.', name: 'queries', need: c => c.queries?.length, message: 'contribute queries'},
  {prefix: 'loom.runtime.', name: 'runtime', need: () => true, message: ''},
];
export const kindOf = id => KINDS.find(kind => id.startsWith(kind.prefix));

/** `id@range` as written in `requires`. */
export function parseRequirement(text) {
  const at = typeof text === 'string' ? text.indexOf('@') : -1;
  if (at <= 0) return null;
  const id = text.slice(0, at), range = text.slice(at + 1);
  return validId(id) && validRange(range) ? {id, range} : null;
}

const WASM_HEADER = Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);

function checkLanguagePack(dir, packPath, problems) {
  const at = path.join(dir, ...packPath.split('/'));
  if (!fs.existsSync(at)) return;
  if (path.posix.basename(packPath) !== 'tree-sitter.json') {
    problems.push(`${packPath}: a language resource must be a tree-sitter.json`);
    return;
  }
  const folder = path.dirname(at);
  let pack;
  try { pack = JSON.parse(fs.readFileSync(at, 'utf8')); } catch (error) { problems.push(`${packPath}: ${error.message}`); return; }
  if (!Array.isArray(pack.grammars) || !pack.grammars.length || pack.grammars.length > 128) {
    problems.push(`${packPath}: needs 1 to 128 grammars`);
    return;
  }
  const seen = new Set();
  for (const grammar of pack.grammars) {
    const name = grammar?.name;
    if (typeof name !== 'string' || !GRAMMAR.test(name) || name === 'plaintext' || seen.has(name)) {
      problems.push(`${packPath}: invalid or duplicate grammar name ${JSON.stringify(name)}`);
      continue;
    }
    seen.add(name);
    // The module is looked for in the grammar's own folder, then beside tree-sitter.json.
    const own = grammar.path && grammar.path !== '.' ? path.join(folder, ...String(grammar.path).split('/')) : folder;
    const module = [own, folder]
      .flatMap(directory => [`tree-sitter-${name}.wasm`, `${name}.wasm`].map(file => path.join(directory, file)))
      .find(file => fs.existsSync(file));
    if (!module) problems.push(`${packPath}: missing WebAssembly module for ${name}`);
    else if (!fs.readFileSync(module).subarray(0, 8).equals(WASM_HEADER)) problems.push(`${packPath}: ${path.basename(module)} is not a WebAssembly module`);
    for (const kind of QUERY_FILES) {
      const declared = grammar[kind === 'highlights.scm' ? 'highlights' : 'injections'];
      const listed = declared === undefined ? [`queries/${kind}`] : [declared].flat();
      for (const file of listed) {
        if (typeof file !== 'string') { problems.push(`${packPath}: ${name} ${kind} must be paths`); continue; }
        const target = path.join(folder, ...file.split('/'));
        let safe = true;
        try { relative(file); } catch { safe = false; }
        if (!safe) problems.push(`${packPath}: ${name}: unsafe query path ${file}`);
        else if (!fs.existsSync(target) && (kind === 'highlights.scm' || declared !== undefined)) problems.push(`${packPath}: ${name}: missing ${file}`);
      }
    }
  }
}

/**
 * Check the extension in `dir`. `built` also checks what a build produces (language
 * server commands that name real files); `publishing` requires the fields people see.
 */
export function checkExtension(dir, {built = false, publishing = true} = {}) {
  const problems = [];
  const file = path.join(dir, 'manifest.json');
  if (!fs.existsSync(file)) return {manifest: null, problems: ['manifest.json is missing']};
  let manifest;
  try { manifest = readJson(file); } catch (error) { return {manifest: null, problems: [`manifest.json is not valid JSON: ${error.message}`]}; }
  const id = manifest.id;
  const add = message => problems.push(message);
  if (manifest.schema !== 1) add('schema must be 1');
  if (!validId(id)) add(`invalid extension id ${JSON.stringify(id)}`);
  else if (path.basename(dir) !== id && !built) add(`the folder is named ${path.basename(dir)} but the id is ${id}`);
  if (!validVersion(manifest.version)) add(`invalid version ${JSON.stringify(manifest.version)}`);
  if (manifest.engines?.loom !== undefined && !validRange(manifest.engines.loom)) add(`invalid engines.loom range ${JSON.stringify(manifest.engines.loom)}`);
  else if (publishing && manifest.engines?.loom === undefined) add('engines.loom is required');
  if (publishing) {
    for (const field of ['name', 'publisher', 'description', 'license']) {
      if (typeof manifest[field] !== 'string' || !manifest[field].trim()) add(`${field} is required`);
    }
    if (!Array.isArray(manifest.categories) || !manifest.categories.length) add('categories is required');
  }
  for (const field of ['icon', 'readme', 'changelog']) {
    if (manifest[field] !== undefined) {
      try { relative(manifest[field], field); if (!fs.existsSync(path.join(dir, ...manifest[field].split('/')))) add(`${field}: ${manifest[field]} does not exist`); }
      catch (error) { add(error.message); }
    }
  }
  for (const capability of manifest.capabilities ?? []) if (!CAPABILITIES.includes(capability)) add(`unknown capability ${capability}`);
  if ((manifest.api === undefined) !== (manifest.component === undefined)) add('api and component must be given together');
  if (manifest.component !== undefined) {
    try { relative(manifest.component, 'component'); if (!fs.existsSync(path.join(dir, ...manifest.component.split('/')))) add(`component ${manifest.component} does not exist`); }
    catch (error) { add(error.message); }
  }
  for (const activation of manifest.activation ?? []) {
    if (activation.kind === 'event' && !EVENTS.includes(activation.name)) add(`unknown event ${activation.name}`);
  }
  for (const text of manifest.requires ?? []) if (!parseRequirement(text)) add(`invalid requirement ${JSON.stringify(text)}; write id@range`);
  for (const text of manifest.recommends ?? []) if (!validId(text)) add(`invalid recommendation ${JSON.stringify(text)}`);

  const contributes = manifest.contributes ?? {};
  const ids = new Set();
  const namespaced = (item, what) => {
    if (typeof item !== 'string' || !validId(id) || !item.startsWith(`${id}.`)) add(`${what} ${JSON.stringify(item)} must start with ${id}.`);
    else if (ids.has(item)) add(`${what} ${item} is declared twice`);
    ids.add(item);
  };
  const resource = (entry, what) => {
    namespaced(entry?.id, what);
    try {
      relative(entry?.path, `${what} path`);
      if (!fs.existsSync(path.join(dir, ...entry.path.split('/')))) add(`${what} ${entry.id}: ${entry.path} does not exist`);
    } catch (error) { add(error.message); }
  };
  for (const kind of ['themes', 'languages', 'snippets']) for (const entry of contributes[kind] ?? []) resource(entry, kind.slice(0, -1));
  for (const entry of contributes.themes ?? []) {
    const target = path.join(dir, ...String(entry.path).split('/'));
    if (fs.existsSync(target)) {
      try { JSON.parse(fs.readFileSync(target, 'utf8')); } catch (error) { add(`theme ${entry.id}: ${error.message}`); }
    }
  }
  for (const entry of contributes.languages ?? []) checkLanguagePack(dir, entry.path, problems);
  for (const query of contributes.queries ?? []) {
    resource(query, 'query');
    if (!GRAMMAR.test(query.grammar ?? '') || !QUERY_FILES.includes(query.file)) add(`query ${query.id}: needs a grammar name and highlights.scm or injections.scm`);
  }
  for (const server of contributes.language_servers ?? []) {
    namespaced(server.id, 'language server');
    if (!Array.isArray(server.languages) || !server.languages.length || !server.languages.every(name => GRAMMAR.test(name))) add(`language server ${server.id}: languages must be canonical names`);
    if (!Array.isArray(server.root_markers) || !server.root_markers.length) add(`language server ${server.id}: root_markers is required`);
    if (!(manifest.capabilities ?? []).includes('process.execute-local')) add(`language server ${server.id}: declare process.execute-local`);
    const command = server.command;
    const match = typeof command === 'string' ? /^\$\{(extension|runtime\.[a-z0-9-]+)\}\/(.+)$/.exec(command) : null;
    if (!match) add(`language server ${server.id}: the command must start with \${extension}/ or \${runtime.<id>}/`);
    else {
      try { relative(match[2], 'command'); } catch (error) { add(error.message); }
      if (built && match[1] === 'extension') {
        const target = path.join(dir, ...match[2].split('/'));
        if (!fs.existsSync(target) && !fs.existsSync(`${target}.exe`)) add(`language server ${server.id}: ${match[2]} is not in the package`);
      }
    }
  }
  const kind = validId(id) ? kindOf(id) : null;
  if (kind && !kind.need(contributes)) add(`an extension named ${kind.prefix}* must ${kind.message}`);
  if (kind?.name === 'language' && !(manifest.recommends ?? []).every(item => item.startsWith('loom.lsp.'))) add('a language extension recommends only language servers (loom.lsp.*)');

  // Grammars run in a sandbox; a native library would not.
  try { for (const name of files(dir)) if (NATIVE.test(name) && kind?.name !== 'language server' && kind?.name !== 'runtime') add(`${name}: native libraries are not allowed in this kind of extension`); }
  catch (error) { add(error.message); }
  return {manifest, problems};
}
