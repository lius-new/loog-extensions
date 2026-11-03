// Development staging uses the same version directories and activation index
// as Loom's installed-package loader. Flat folders alone are not discoverable.
import fs from 'node:fs';
import path from 'node:path';
import {createHash, randomUUID} from 'node:crypto';

export function developmentDirectory(root, manifest) {
  if (!/^[a-z0-9][a-z0-9._-]*$/.test(manifest.id)
    || !/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?(?:\+[a-zA-Z0-9.-]+)?$/.test(manifest.version)) {
    throw new Error('Invalid development package id or version');
  }
  return path.join(path.resolve(root), manifest.id, manifest.version);
}

function fingerprint(directory) {
  const hash = createHash('sha256');
  const visit = folder => {
    for (const entry of fs.readdirSync(folder, {withFileTypes: true}).sort((a, b) => a.name.localeCompare(b.name))) {
      const file = path.join(folder, entry.name);
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) {
        hash.update(path.relative(directory, file).split(path.sep).join('/') + '\0');
        hash.update(fs.readFileSync(file));
        hash.update('\0');
      } else throw new Error(`Unsupported development package entry: ${file}`);
    }
  };
  visit(directory);
  return hash.digest('hex');
}

function validSource(source) {
  if (!source || typeof source !== 'object' || Array.isArray(source)
    || typeof source.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(source.sha256)) return false;
  switch (source.kind) {
    case 'file': return source.file == null || typeof source.file === 'string';
    case 'index': return typeof source.url === 'string';
    case 'registry': return typeof source.release === 'string';
    default: return false;
  }
}

export function registerDevelopment(root, packages) {
  const stateFile = path.join(root, 'extensions.json');
  const state = fs.existsSync(stateFile) ? JSON.parse(fs.readFileSync(stateFile, 'utf8')) : {extensions: {}, disabled: []};
  if (!state.extensions || typeof state.extensions !== 'object' || Array.isArray(state.extensions)
    || !Array.isArray(state.disabled)) throw new Error('Invalid development extensions.json');
  for (const {directory, source} of packages) {
    const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
    if (path.resolve(directory) !== developmentDirectory(root, manifest)) {
      throw new Error(`Development package must be staged under <id>/<version>: ${directory}`);
    }
    // Unpacked local snapshots have no archive path. This records their source
    // digest; Loom independently derives execution identities from package bytes.
    const localSource = source ?? {kind: 'file', sha256: fingerprint(directory)};
    if (!validSource(localSource)) {
      throw new Error(`Invalid development source for ${manifest.id}: expected a tagged source record, not a path`);
    }
    const previous = state.extensions[manifest.id];
    state.extensions[manifest.id] = {
      version: manifest.version,
      pinned: true,
      source: localSource,
      ...(previous?.version !== undefined && previous.version !== manifest.version
        ? {previous: previous.version} : previous?.previous ? {previous: previous.previous} : {}),
    };
  }
  // Loom rejects the entire activation index if any Source enum is malformed.
  // Validate every retained entry as well as the entries changed by this batch.
  for (const [id, entry] of Object.entries(state.extensions)) {
    if (!entry || typeof entry.version !== 'string' || !validSource(entry.source)
      || (entry.pinned != null && typeof entry.pinned !== 'boolean')
      || (entry.previous != null && typeof entry.previous !== 'string')) {
      throw new Error(`Invalid activation record for ${id}; extensions.json was not changed`);
    }
  }
  if (state.disabled.some(id => typeof id !== 'string')) throw new Error('Invalid disabled extension ids');
  fs.mkdirSync(root, {recursive: true});
  const temporary = path.join(root, `.extensions-${randomUUID()}.tmp`);
  try {
    fs.writeFileSync(temporary, JSON.stringify(state, null, 2) + '\n');
    fs.renameSync(temporary, stateFile);
  } finally {
    if (fs.existsSync(temporary)) fs.unlinkSync(temporary);
  }
}
