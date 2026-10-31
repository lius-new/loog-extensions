import test from 'node:test';
import assert from 'node:assert/strict';
import {compare, satisfies, validRange, validVersion} from '../lib/semver.mjs';

test('versions order by number, then pre-release', () => {
  assert.ok(compare('1.2.3', '1.2.4') < 0);
  assert.ok(compare('1.10.0', '1.9.9') > 0);
  assert.ok(compare('1.0.0-alpha', '1.0.0') < 0);
  assert.ok(compare('1.0.0-alpha.2', '1.0.0-alpha.10') < 0);
  assert.equal(compare('1.0.0+build', '1.0.0'), 0);
  assert.ok(validVersion('0.1.8') && !validVersion('1.0') && !validVersion('v1.0.0'));
});

test('ranges: bounds, carets, tildes and wildcards', () => {
  const yes = (version, range) => assert.ok(satisfies(version, range), `${version} in ${range}`);
  const no = (version, range) => assert.ok(!satisfies(version, range), `${version} not in ${range}`);
  yes('0.1.8', '>=0.1.8 <0.3');
  no('0.3.0', '>=0.1.8 <0.3');
  no('0.1.7', '>=0.1.8 <0.3');
  yes('1.4.0', '^1.2.3');
  no('2.0.0', '^1.2.3');
  yes('0.2.9', '^0.2.3');
  no('0.3.0', '^0.2.3');
  no('0.0.4', '^0.0.3');
  yes('1.2.9', '~1.2.3');
  no('1.3.0', '~1.2.3');
  yes('22.3.1', '^22');
  no('23.0.0', '^22');
  yes('1.9.0', '1.x');
  yes('3.0.0', '*');
  yes('1.2.3', '1.2.3');
  no('1.2.4', '1.2.3');
  yes('1.2.5', '1.2');
  yes('0.1.0', '<0.2 || >=1.0');
  yes('1.1.0', '<0.2 || >=1.0');
  no('0.5.0', '<0.2 || >=1.0');
  yes('1.3.0', '>1.2');
  no('1.2.9', '>1.2');
});

test('nonsense ranges are rejected', () => {
  assert.ok(!validRange('>=a.b'));
  assert.ok(!validRange('1.2.3.4'));
  assert.ok(validRange('>=0.1.8 <0.3'));
});
