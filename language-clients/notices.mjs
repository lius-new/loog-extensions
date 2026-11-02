// Retain source/runtime notices beside each packaged component.
import fs from 'node:fs';
import path from 'node:path';

export function clientNotices(folder) {
  const loom = path.resolve(process.env.LOOM_SOURCE ?? path.join(import.meta.dirname, '../../..'));
  const licenses = path.join(folder, 'licenses');
  fs.mkdirSync(licenses, {recursive:true});
  for (const name of ['LICENSE-MIT', 'LICENSE-APACHE']) {
    fs.copyFileSync(path.join(import.meta.dirname, '..', name), path.join(licenses, `loom-client-${name}`));
  }
  fs.copyFileSync(path.join(loom, 'tests/extensions/js-guest/node_modules/@bytecodealliance/componentize-js/LICENSE'), path.join(licenses, 'component-runtime'));
  fs.writeFileSync(path.join(licenses, 'CLIENT-NOTICES.txt'),
    'Loom extension client and general SDK: MIT OR Apache-2.0.\n' +
    'Sources: https://github.com/lius-new/loom-extensions/tree/main/language-clients and https://github.com/lius-new/loom/tree/main/sdk\n' +
    'Embedded runtime: componentize-js / StarlingMonkey (Apache-2.0 WITH LLVM-exception); includes SpiderMonkey.\n' +
    'Runtime sources and additional notices: https://github.com/bytecodealliance/ComponentizeJS and https://github.com/bytecodealliance/StarlingMonkey\n');
}
