// Plan, prepare and publish all explicitly versioned extensions missing from the index.
import fs from 'node:fs';
import path from 'node:path';
import {ROOT, WORK, assert, readJson, removeUnder, writeAtomic} from '../lib/util.mjs';
import {loadConfig} from '../lib/config.mjs';
import {clearStage} from '../lib/stage.mjs';
import {findValidator} from '../lib/validator.mjs';
import {definitions, planRelease, changedExtensions, revision, readRegistry, prepareRelease, finishRelease, releaseDirectory, checkSigningKey} from '../lib/release.mjs';
import {run as lock} from './lock.mjs';

export const usage = 'release <plan|preflight|prepare|publish>    Plan pending versions, validate packages and publish automatically';
const directory = releaseDirectory();
const planFile = path.join(directory, 'plan.json'), outcomesFile = path.join(directory, 'prepared.json');

function summary(text) {
  console.log(text);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, text + '\n');
}

export async function run(argv) {
  assert(argv.length === 1 && ['plan', 'preflight', 'prepare', 'publish'].includes(argv[0]), usage);
  const config = loadConfig();
  if (argv[0] === 'preflight') { checkSigningKey(config, process.env.LOOM_INDEX_KEY); return; }
  if (argv[0] === 'plan') {
    const commit = revision();
    const event = process.env.GITHUB_EVENT_PATH ? readJson(process.env.GITHUB_EVENT_PATH) : {};
    const base = event.pull_request ? event.pull_request.base.sha : event.before;
    const head = event.pull_request ? event.pull_request.head.sha : commit;
    // A PR's merge base excludes target-branch-only changes.
    let comparison = base;
    if (event.pull_request) {
      const {execFileSync} = await import('node:child_process');
      comparison = execFileSync('git', ['merge-base', base, head], {cwd: ROOT, encoding: 'utf8'}).trim();
    }
    const current = await readRegistry(config);
    const plan = {...planRelease(definitions(), current.index, changedExtensions(ROOT, comparison, head), commit), repair: current.recovered};
    writeAtomic(planFile, JSON.stringify(plan, null, 2) + '\n');
    summary(`Pending versions: ${plan.pending.length}; already indexed: ${plan.skipped.length}.\n\n` + plan.pending.map(e => `- ${e.id} ${e.version} (${e.platforms.join(', ')})`).join('\n'));
    if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `count=${plan.pending.length}\nrepair=${plan.repair}\n`);
    return;
  }
  const plan = readJson(planFile);
  assert(plan.schema === 1 && plan.revision === revision(), 'Release plan is not for this checkout');
  if (!plan.pending.length && !plan.repair) { summary('No pending versions; nothing was changed.'); return; }
  if (argv[0] === 'prepare') {
    if (!plan.pending.length) { writeAtomic(outcomesFile, '[]\n'); return; }
    const validator = findValidator();
    assert(validator, 'Automatic releases require loom-ext-validate');
    const results = [];
    for (const item of plan.pending) {
      try {
        results.push(await prepareRelease(item, {validator, repository: config.repository}));
        summary(`Prepared ${item.id} ${item.version}`);
      } catch (error) {
        results.push({id: item.id, error: error.message});
        summary(`Failed ${item.id}: ${error.message}`);
      }
      writeAtomic(outcomesFile, JSON.stringify(results, null, 2) + '\n');
      // The immutable packages and recorded descriptions are all publication needs.
      clearStage(item.id);
      removeUnder(WORK, path.join(WORK, 'extracted'));
    }
    return;
  }
  const {execFileSync} = await import('node:child_process');
  assert(!execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], {cwd: ROOT, encoding: 'utf8'}).trim(), 'Commit the working tree before publishing; release tags must describe the tested source');
  const report = await finishRelease(plan, readJson(outcomesFile), {config, privateKey: process.env.LOOM_INDEX_KEY, sourceRevision: revision()});
  summary(`Indexed: ${report.indexed.length}; failures: ${report.failures.length}; sequence: ${report.sequence}.\n\n` + report.failures.map(e => `- ${e.id}: ${e.error}`).join('\n'));
  assert(!report.failures.length, 'Some extensions were not delivered; see the release report. Rerun to complete them.');
  await lock([]);
}
