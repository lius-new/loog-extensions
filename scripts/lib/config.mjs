// loog-ext.toml, with the checks that keep a typo from publishing to the wrong place.
import fs from 'node:fs';
import path from 'node:path';
import {ROOT, assert, readToml} from './util.mjs';
import {validId} from './manifest.mjs';

export function loadConfig(root = ROOT) {
  const file = path.join(root, 'loog-ext.toml');
  assert(fs.existsSync(file), 'loog-ext.toml is missing');
  const config = readToml(file);
  assert(config.schema === 1, 'loog-ext.toml: schema must be 1');
  assert(/^[\w.-]+\/[\w.-]+$/.test(config.repository ?? ''), 'loog-ext.toml: repository must be owner/name');
  assert(Array.isArray(config.bundled) && config.bundled.every(validId), 'loog-ext.toml: bundled must list extension ids');
  assert(new Set(config.bundled).size === config.bundled.length, 'loog-ext.toml: bundled lists an extension twice');
  config.deferred ??= [];
  assert(Array.isArray(config.deferred) && config.deferred.every(validId), 'loog-ext.toml: deferred must list extension ids');
  assert(new Set(config.deferred).size === config.deferred.length, 'loog-ext.toml: deferred lists an extension twice');
  assert(!config.deferred.some(id => config.bundled.includes(id)), 'loog-ext.toml: a bundled extension cannot be deferred');
  const index = config.index ?? {};
  assert(typeof index['key-id'] === 'string' && /^[\w.-]+$/.test(index['key-id']), 'loog-ext.toml: index.key-id is required');
  assert(Number.isInteger(index['validity-days']) && index['validity-days'] > 0, 'loog-ext.toml: index.validity-days must be a positive number');
  return config;
}
