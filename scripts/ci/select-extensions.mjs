// Choose one build matrix from the full git diff, without GitHub's path-filter limit.
import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {loadConfig} from '../lib/config.mjs';
import clients from '../../language-clients/rust/clients.json' with {type: 'json'};

const ROOT = fileURLToPath(new URL('../../', import.meta.url));
const shared = file => file.startsWith('.github/workflows/') || file.startsWith('scripts/') ||
  file.startsWith('tools/') || ['package.json', 'package-lock.json', 'loog-ext.toml'].includes(file);

export function selectExtensions(available, files, {all = false, requested = ''} = {}) {
  const ids = [...new Set(available)].sort();
  if (requested.trim()) {
    const selected = [...new Set(requested.split(',').map(id => id.trim()))];
    for (const id of selected) if (!ids.includes(id)) throw new Error('Unknown extension: ' + id);
    return selected.sort();
  }
  if (all || files.some(shared)) return ids;
  const changed = new Set(files.filter(file => file.startsWith('extensions/')).map(file => file.split('/')[1]));
  if (files.some(file => file.startsWith('language-clients/'))) {
    for (const id of Object.keys(clients)) changed.add(id);
  }
  return ids.filter(id => changed.has(id));
}

export function changedFiles(eventName, event, root = ROOT) {
  if (eventName === 'workflow_dispatch') return {all: true, files: []};
  const git = args => execFileSync('git', args, {cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe']});
  const base = eventName === 'pull_request' ? event.pull_request?.base?.sha : event.before;
  if (eventName !== 'push' && eventName !== 'pull_request') throw new Error('Unsupported event: ' + eventName);
  if (eventName === 'push' && (!base || /^0+$/.test(base))) return {all: true, files: []};
  if (!/^[a-f0-9]{40}$/.test(base ?? '')) throw new Error('Missing or invalid comparison commit');
  // A force push may make the old tip unreachable even with fetch-depth: 0.
  if (eventName === 'push') {
    try { git(['cat-file', '-e', base + '^{commit}']); }
    catch { return {all: true, files: []}; }
  }
  const range = eventName === 'pull_request' ? [base + '...HEAD'] : [base, 'HEAD'];
  const files = git(['diff', '--no-renames', '--name-only', '-z', ...range, '--']).split('\0').filter(Boolean);
  return {all: false, files};
}

export function run(env = process.env, root = ROOT) {
  const deferred = fs.existsSync(path.join(root, 'loog-ext.toml')) ? loadConfig(root).deferred : [];
  const available = fs.readdirSync(path.join(root, 'extensions'), {withFileTypes: true})
    .filter(entry => entry.isDirectory() && /^loog\.[a-z0-9]+(?:[.-][a-z0-9]+)*$/.test(entry.name))
    .map(entry => entry.name).filter(id => !deferred.includes(id));
  const event = JSON.parse(fs.readFileSync(env.GITHUB_EVENT_PATH, 'utf8'));
  const changes = changedFiles(env.GITHUB_EVENT_NAME, event, root);
  const selected = selectExtensions(available, changes.files, {...changes,
    requested: env.GITHUB_EVENT_NAME === 'workflow_dispatch' ? env.REQUESTED_EXTENSIONS ?? '' : ''});
  const output = 'matrix=' + JSON.stringify({extension: selected}) + '\n' +
    'has-extensions=' + Boolean(selected.length) + '\n';
  fs.appendFileSync(env.GITHUB_OUTPUT, output);
  console.log('Selected ' + selected.length + ' extensions: ' + (selected.join(', ') || '(none)'));
  return selected;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) run();
