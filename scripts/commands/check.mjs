// check <package or folder…>: what intake checks, without publishing anything. Each
// package is unpacked and must be one Loog can import; how it was built is not looked at.
import {assert} from '../lib/util.mjs';
import {findValidator} from '../lib/validator.mjs';
import {intake, packageFiles} from '../lib/intake.mjs';
import {parseArgs} from '../lib/cli.mjs';

export const usage = 'check <package or folder…>    Check finished packages the way publishing does (imports them with loog-ext-validate when found)';

export async function run(argv) {
  const {ids: paths} = parseArgs(argv, {});
  assert(paths.length, 'Name the .loogext packages, or the folders holding them');
  const validator = findValidator();
  if (!validator) console.warn('loog-ext-validate was not found; only the structure of the packages is checked.');
  const results = await intake(packageFiles(paths), {validator});
  for (const result of results) console.log(result.error ? `Rejected ${result.id}: ${result.error}` : `Checked ${result.id} ${result.version} (${result.records.map(record => record.platform).join(', ')})`);
  assert(!results.some(result => result.error), 'Some packages were rejected');
}
