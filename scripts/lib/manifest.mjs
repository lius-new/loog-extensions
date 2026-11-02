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
// The categories an extension may list, at most three; and the size limits for its documents.
export const CATEGORIES = ['Themes', 'Syntax Highlighting', 'Language Services', 'Snippets', 'Linters', 'Formatters', 'Editing', 'Source control', 'Productivity', 'Other'];
export const MAX_ICON_BYTES = 256 * 1024;
export const MAX_DOCUMENT_BYTES = 512 * 1024;
export const EVENTS = ['document.open', 'document.change', 'document.close', 'document.save',
  'selection.change', 'focus.change', 'root.change', 'workspace.files'];

const SEGMENT = /^[a-z0-9][a-z0-9-]*$/;
export const validId = id => typeof id === 'string' && id.split('.').length >= 2 && id.split('.').every(part => SEGMENT.test(part));
const GRAMMAR = /^[a-z][a-z0-9_]*$/;
const NATIVE = /\.(dll|so|dylib)$/i;
const QUERY_FILES = ['highlights.scm', 'injections.scm'];

/**
 * The contribution points of a manifest, each named by what it is for: highlight
 * and highlight_queries colour syntax, language_service answers hints and
 * diagnostics, themes are colour themes. Keep in step with `Contributes` in Loom's
 * src/features/extensions/manifest.rs, which refuses any other name.
 */
export const CONTRIBUTIONS = ['commands', 'themes', 'snippets', 'languages', 'highlight', 'highlight_queries', 'language_service', 'settings'];
export const LANGUAGE_SERVICE = ['servers', 'providers'];
export const PROVIDER_KINDS = ['completion', 'diagnostics', 'formatting', 'code-actions', 'save-participant', 'hover', 'definition', 'document-symbols', 'signature-help', 'inlay-hints', 'references', 'workspace-symbols', 'document-highlights', 'rename', 'folding-ranges', 'selection-ranges', 'call-hierarchy', 'type-hierarchy', 'code-lens', 'format-on-type', 'execute-command', 'highlights'];

/** What a package of this id must contribute, by the naming scheme. */
export const KINDS = [
  {prefix: 'loom.lang.', name: 'highlight', need: c => c.highlight?.length, message: 'contribute highlight'},
  {prefix: 'loom.lsp.', name: 'language service', need: c => c.language_service?.providers?.length || c.language_service?.servers?.length, message: 'declare language service providers'},
  {prefix: 'loom.theme.', name: 'theme', need: c => c.themes?.length, message: 'contribute a theme'},
  {prefix: 'loom.queries.', name: 'highlight queries', need: c => c.highlight_queries?.length, message: 'contribute highlight queries'},
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
    problems.push(`${packPath}: a highlight resource must be a tree-sitter.json`);
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
  // What people see. Loom checks these when it loads a package; publishing also needs them all.
  for (const [field, max] of [['name', 60], ['publisher', 60], ['description', 200], ['license', 100]]) {
    const value = manifest[field];
    if (value === undefined) continue;
    if (typeof value !== 'string' || !value.trim() || [...value].length > max) add(`${field} must be 1 to ${max} characters`);
    else if (/[\n\r\0]/.test(value)) add(`${field} must be a single line`);
  }
  const categories = manifest.categories ?? [];
  if (!Array.isArray(categories) || categories.length > 3) add('categories lists more than three');
  else for (const category of categories) if (!CATEGORIES.includes(category)) add(`${JSON.stringify(category)} is not a category; use one of ${CATEGORIES.join(', ')}`);
  const keywords = manifest.keywords ?? [];
  if (!Array.isArray(keywords) || keywords.length > 10 || keywords.some(word => typeof word !== 'string' || !word.trim() || [...word].length > 30)) {
    add('keywords allows at most ten words of up to 30 characters');
  }
  for (const field of ['repository', 'homepage']) {
    const url = manifest[field];
    if (url !== undefined && (typeof url !== 'string' || !url.startsWith('https://') || url.length > 300 || /\s/.test(url))) add(`${field} must be an https:// address`);
  }
  const DOCUMENTS = {icon: {suffix: /\.(png|svg)$/i, limit: MAX_ICON_BYTES}, readme: {suffix: /\.md$/i, limit: MAX_DOCUMENT_BYTES}, changelog: {suffix: /\.md$/i, limit: MAX_DOCUMENT_BYTES}};
  for (const [field, {suffix, limit}] of Object.entries(DOCUMENTS)) {
    if (manifest[field] === undefined) continue;
    try {
      relative(manifest[field], field);
      if (!suffix.test(manifest[field])) add(`${field} must be ${field === 'icon' ? 'a PNG or SVG file' : 'a Markdown file'}`);
      const target = path.join(dir, ...manifest[field].split('/'));
      if (!fs.existsSync(target)) add(`${field}: ${manifest[field]} does not exist`);
      else if (!fs.statSync(target).size) add(`${field}: ${manifest[field]} is empty`);
      else if (fs.statSync(target).size > limit) add(`${field}: ${manifest[field]} is larger than ${limit / 1024} KiB`);
    } catch (error) { add(error.message); }
  }
  if (publishing) {
    for (const field of ['name', 'publisher', 'description', 'license', 'repository', 'icon', 'readme', 'changelog']) {
      if (manifest[field] === undefined) add(`${field} is required`);
    }
    if (!categories.length) add('categories is required');
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
  // Loom refuses a manifest with a contribution point it does not know, so a package
  // written with an old or misspelled name is caught before it is published.
  for (const key of Object.keys(contributes)) {
    if (!CONTRIBUTIONS.includes(key)) add(`contributes.${key} is not a contribution point; use one of ${CONTRIBUTIONS.join(', ')}`);
  }
  for (const key of Object.keys(contributes.language_service ?? {})) {
    if (!LANGUAGE_SERVICE.includes(key)) add(`contributes.language_service.${key} is not part of the language service; use ${LANGUAGE_SERVICE.join(' or ')}`);
  }
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
  for (const [kind, what] of [['themes', 'theme'], ['highlight', 'highlight'], ['languages', 'language identity'], ['snippets', 'snippet']]) {
    for (const entry of contributes[kind] ?? []) resource(entry, what);
  }
  for (const entry of contributes.themes ?? []) {
    const target = path.join(dir, ...String(entry.path).split('/'));
    if (fs.existsSync(target)) {
      try { JSON.parse(fs.readFileSync(target, 'utf8')); } catch (error) { add(`theme ${entry.id}: ${error.message}`); }
    }
  }
  for (const entry of contributes.highlight ?? []) checkLanguagePack(dir, entry.path, problems);
  for (const entry of contributes.languages ?? []) {
    try {
      relative(entry.path, 'language identity path');
      if (path.posix.basename(entry.path) !== 'language.json') throw new Error('must name language.json');
      const target = path.join(dir, ...entry.path.split('/'));
      if (!fs.existsSync(target)) continue;
      if (fs.statSync(target).size > 1024 * 1024) throw new Error('exceeds 1 MiB');
      const value = readJson(target);
      if (!GRAMMAR.test(value.name ?? '') || value.name === 'plaintext') throw new Error('invalid canonical language name');
      if (!Array.isArray(value.variants) || !value.variants.length || value.variants.some(variant => !GRAMMAR.test(variant.id ?? '') || Object.hasOwn(variant, 'grammar'))) throw new Error('variants must declare identities without parser bindings');
      const variants = new Set(value.variants.map(variant => variant.id));
      if (variants.size !== value.variants.length) throw new Error('duplicate variant');
      if (!Array.isArray(value.files) || value.files.some(file => !variants.has(file.variant))) throw new Error('file associations must name a declared variant');
    } catch (error) { add(`language identity ${entry.id}: ${error.message}`); }
  }
  for (const query of contributes.highlight_queries ?? []) {
    resource(query, 'highlight query');
    if (!GRAMMAR.test(query.grammar ?? '') || !QUERY_FILES.includes(query.file)) add(`highlight query ${query.id}: needs a grammar name and highlights.scm or injections.scm`);
  }
  // Files that must be runnable: the application sets the execute bit on exactly these.
  if (manifest.executables !== undefined) {
    if (!Array.isArray(manifest.executables)) add('executables must be a list of paths');
    else for (const executable of manifest.executables) {
      try {
        relative(executable, 'executable');
        const target = path.join(dir, ...executable.split('/'));
        if (built && !fs.existsSync(target) && !fs.existsSync(`${target}.exe`)) add(`executable ${executable} is not in the package`);
      } catch (error) { add(error.message); }
    }
  }
  for (const provider of contributes.language_service?.providers ?? []) {
    namespaced(provider.id, 'provider');
    if (!PROVIDER_KINDS.includes(provider.kind)) add(`provider ${provider.id}: unknown kind ${provider.kind}`);
    if (!manifest.api || !manifest.component) add(`provider ${provider.id}: requires api and component`);
    if (!(manifest.capabilities ?? []).includes('documents.read') || !(manifest.capabilities ?? []).includes('languages')) add(`provider ${provider.id}: declare documents.read and languages`);
    if (!Array.isArray(provider.selector?.languages) || !provider.selector.languages.length || !provider.selector.languages.every(name => GRAMMAR.test(name))) add(`provider ${provider.id}: languages must be canonical names`);
  }
  for (const server of contributes.language_service?.servers ?? []) {
    namespaced(server.id, 'language server');
    if (server.root_strategy !== undefined && !['nearest', 'outermost'].includes(server.root_strategy)) add(`language server ${server.id}: invalid root_strategy`);
    if (server.root_fallback !== undefined && !['workspace', 'file_directory', 'none'].includes(server.root_fallback)) add(`language server ${server.id}: invalid root_fallback`);
    if (server.priority !== undefined && !Number.isSafeInteger(server.priority)) add(`language server ${server.id}: invalid priority`);
    for (const binding of server.bindings ?? []) {
      if (!GRAMMAR.test(binding.language ?? '') || (binding.variant !== undefined && !GRAMMAR.test(binding.variant)) || typeof binding.language_id !== 'string' || !binding.language_id) {
        add(`language server ${server.id}: invalid language binding`);
      }
    }
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
  if (kind?.name === 'highlight' && !(manifest.recommends ?? []).every(item => item.startsWith('loom.lsp.'))) add('a highlight extension recommends only language service extensions (loom.lsp.*)');

  // Grammars run in a sandbox; a native library would not.
  try { for (const name of files(dir)) if (NATIVE.test(name) && kind?.name !== 'language service' && kind?.name !== 'runtime') add(`${name}: native libraries are not allowed in this kind of extension`); }
  catch (error) { add(error.message); }
  return {manifest, problems};
}
