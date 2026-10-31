// A tiny argument parser: `--flag`, `--name value` and positional extension ids.
import {assert} from './util.mjs';

/** `spec` maps an option name to `'boolean'` or `'string'`. */
export function parseArgs(argv, spec) {
  const options = {};
  const ids = [];
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith('--')) { ids.push(arg); continue; }
    const name = arg.slice(2);
    assert(spec[name], `Unknown option: ${arg}`);
    if (spec[name] === 'boolean') options[name] = true;
    else {
      assert(argv[i + 1] !== undefined && !argv[i + 1].startsWith('--'), `Missing value: ${arg}`);
      options[name] = argv[++i];
    }
  }
  return {options, ids};
}

export function main(callback) {
  callback().catch(error => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
