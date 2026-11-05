// The pinned Loom source commit that builds and verification run against.
import path from 'node:path';
import {assert, ROOT, readToml} from './util.mjs';

export function readLoomSource() {
  const source = readToml(path.join(ROOT, 'tools/loom-source.toml'));
  assert(source.schema === 1 && /^[\w.-]+\/[\w.-]+$/.test(source.repository) && /^[a-f0-9]{40}$/.test(source.revision),
    'Pin the Loom repository and complete source commit in tools/loom-source.toml');
  return {repository: source.repository, revision: source.revision};
}
