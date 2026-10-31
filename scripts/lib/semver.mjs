// Versions and ranges, the subset extensions need: `1.2.3`, `>=1 <2`, `^1.2`,
// `~1.2.3`, `1.x`, `*`, and `||` for alternatives.
import {assert} from './util.mjs';

const VERSION = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/;

export function parseVersion(text) {
  const match = VERSION.exec(text);
  assert(match, `Invalid version: ${text}`);
  return {major: +match[1], minor: +match[2], patch: +match[3], pre: match[4] ? match[4].split('.') : []};
}
export const validVersion = text => VERSION.test(text);

function comparePre(a, b) {
  if (!a.length || !b.length) return a.length === b.length ? 0 : a.length ? -1 : 1;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] === undefined) return -1;
    if (b[i] === undefined) return 1;
    const x = /^\d+$/.test(a[i]), y = /^\d+$/.test(b[i]);
    if (x && y && +a[i] !== +b[i]) return +a[i] < +b[i] ? -1 : 1;
    if (x !== y) return x ? -1 : 1;
    if (!x && a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return 0;
}
/** Negative, zero or positive, as for sorting. */
export function compare(a, b) {
  const x = typeof a === 'string' ? parseVersion(a) : a, y = typeof b === 'string' ? parseVersion(b) : b;
  for (const key of ['major', 'minor', 'patch']) if (x[key] !== y[key]) return x[key] < y[key] ? -1 : 1;
  return comparePre(x.pre, y.pre);
}

const v = (major, minor = 0, patch = 0) => ({major, minor, patch, pre: []});
const lower = (op, version) => ({op, version});

// `1`, `1.2`, `1.x`, `1.2.3` with the parts that were given.
function partial(text) {
  const parts = text.split('.');
  assert(parts.length >= 1 && parts.length <= 3, `Invalid version in range: ${text}`);
  const numbers = [];
  for (const part of parts) {
    if (/^[xX*]$/.test(part)) break;
    assert(/^\d+$/.test(part), `Invalid version in range: ${text}`);
    numbers.push(+part);
  }
  return numbers;
}
const upTo = (numbers, step) => {
  // The exclusive upper bound that bumps the `step`th part and zeroes the rest.
  const next = [...numbers.slice(0, step), (numbers[step] ?? 0) + 1];
  return v(...next);
};

function expand(op, text) {
  const numbers = partial(text);
  if (!numbers.length) return op === '' || op === '=' || op === '>=' || op === '<=' ? [] : [lower('<', v(0))];
  const full = v(...numbers);
  const last = numbers.length - 1;
  switch (op) {
    case '^': {
      const first = numbers.findIndex(n => n !== 0);
      const step = first === -1 ? last : first;
      return [lower('>=', full), lower('<', upTo(numbers, step))];
    }
    case '~': return [lower('>=', full), lower('<', upTo(numbers, numbers.length === 1 ? 0 : 1))];
    case '': case '=': return numbers.length === 3 ? [lower('=', full)] : [lower('>=', full), lower('<', upTo(numbers, last))];
    case '>=': return [lower('>=', full)];
    case '>': return numbers.length === 3 ? [lower('>', full)] : [lower('>=', upTo(numbers, last))];
    case '<': return [lower('<', full)];
    case '<=': return numbers.length === 3 ? [lower('<=', full)] : [lower('<', upTo(numbers, last))];
    default: throw new Error(`Invalid range operator: ${op}`);
  }
}

/** A range as alternatives, each a list of comparators that must all hold. */
export function parseRange(text) {
  assert(typeof text === 'string', 'A version range must be text');
  return text.split('||').map(alternative => {
    const comparators = [];
    for (const token of alternative.trim().split(/\s+/).filter(Boolean)) {
      const match = /^(\^|~|>=|<=|>|<|=)?(.+)$/.exec(token);
      assert(match, `Invalid range: ${text}`);
      comparators.push(...expand(match[1] ?? '', match[2]));
    }
    return comparators;
  });
}
export const validRange = text => { try { parseRange(text); return true; } catch { return false; } };

export function satisfies(version, range) {
  const candidate = parseVersion(version);
  return parseRange(range).some(comparators => comparators.every(({op, version: bound}) => {
    const order = compare(candidate, bound);
    return {'>=': order >= 0, '>': order > 0, '<=': order <= 0, '<': order < 0, '=': order === 0}[op];
  }));
}
