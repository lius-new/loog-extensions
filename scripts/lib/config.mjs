// loom-ext.toml, with the checks that keep a typo from publishing to the wrong place.
import fs from 'node:fs';
import path from 'node:path';
import {ROOT, assert, readToml} from './util.mjs';
import {validId} from './manifest.mjs';

export function loadConfig(root = ROOT) {
  const file = path.join(root, 'loom-ext.toml');
  assert(fs.existsSync(file), 'loom-ext.toml is missing');
  const config = readToml(file);
  assert(config.schema === 1, 'loom-ext.toml: schema must be 1');
  assert(/^[\w.-]+\/[\w.-]+$/.test(config.repository ?? ''), 'loom-ext.toml: repository must be owner/name');
  assert(Array.isArray(config.bundled) && config.bundled.every(validId), 'loom-ext.toml: bundled must list extension ids');
  assert(new Set(config.bundled).size === config.bundled.length, 'loom-ext.toml: bundled lists an extension twice');
  config.deferred ??= [];
  assert(Array.isArray(config.deferred) && config.deferred.every(validId), 'loom-ext.toml: deferred must list extension ids');
  assert(new Set(config.deferred).size === config.deferred.length, 'loom-ext.toml: deferred lists an extension twice');
  assert(!config.deferred.some(id => config.bundled.includes(id)), 'loom-ext.toml: a bundled extension cannot be deferred');
  const index = config.index ?? {};
  assert(typeof index['key-id'] === 'string' && /^[\w.-]+$/.test(index['key-id']), 'loom-ext.toml: index.key-id is required');
  assert(Number.isInteger(index['validity-days']) && index['validity-days'] > 0, 'loom-ext.toml: index.validity-days must be a positive number');
  return config;
}
