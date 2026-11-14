// Both Rust dependencies and WIT must come from the same SDK checkout. CI checks
// its immutable pin out at this location; local builds may link an existing tree.
import fs from 'node:fs';
import path from 'node:path';

export function prepareSdk(root, source) {
  const directory = path.resolve(root, '.work/loog-sdk');
  const selected = path.resolve(source ?? directory);
  for (const file of ['wit/loog-extension/world.wit', 'crates/workspace-api/Cargo.toml']) {
    if (!fs.existsSync(path.join(selected, file)))
      throw new Error('Use the matching Loog development SDK with workspace support; missing ' + file);
  }
  if (fs.existsSync(directory)) {
    if (fs.realpathSync.native(directory) !== fs.realpathSync.native(selected))
      throw new Error('LOOG_SOURCE and .work/loog-sdk name different SDK checkouts; select one consistent checkout');
  } else {
    // Never overwrite or retarget a pre-existing checkout or dangling link.
    fs.mkdirSync(path.dirname(directory), {recursive: true});
    fs.symlinkSync(selected, directory, process.platform === 'win32' ? 'junction' : 'dir');
  }
  return directory;
}
