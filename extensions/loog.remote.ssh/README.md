# Remote SSH (development)

The component owns SSH connection settings, connection UI, the bundled Linux
Python Agent and a Linux x86_64 search executable. Loog supplies generic window commands, user tools and asynchronous
resource operations; it does not recognize SSH or this package's ID.

Build with LOOG_SOURCE pointing to the matching Loog checkout:

~~~powershell
node scripts/sdk.mjs
rustup target add wasm32-unknown-unknown x86_64-unknown-linux-musl
node --input-type=module -e "import {buildSearch} from './remote/search/build.mjs'; buildSearch();"
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path extensions/loog.remote.ssh/rust/Cargo.toml
cargo run --locked --release --manifest-path language-clients/rust/componentize/Cargo.toml -- extensions/loog.remote.ssh/rust/target/wasm32-unknown-unknown/release/loog_remote_ssh.wasm extensions/loog.remote.ssh/extension.wasm
~~~

`scripts/sdk.mjs` links the selected `LOOG_SOURCE` checkout at `.work/loog-sdk`,
the same location CI checks out its immutable SDK pin. It never retargets an
existing SDK. Both Cargo API dependencies and WIT use that checkout; the plugin
repository does not need to live inside the Loog repository. For an installable
development archive, run `npm ci` and `node build.mjs` in this extension folder.

Everything happens in the command palette. Type `ssh` to list the saved hosts
and choose one to connect to it. **Remote SSH: Connect to New Host…** takes an
SSH alias or `user@host`, with `:port` for a port that is not 22, then asks how to
select the private key: **Default Location** (the keys in `~/.ssh` and those named
in the SSH configuration, preselected) or **Choose Key File…**, which opens the
system file dialog and offers only that key. A host that
connected is saved and opens its folder next time; a new host opens the login
folder. Port, jump hosts and other options belong in the SSH configuration. The
palette shows a turning mark while connecting and a cross when it failed; pressing
Enter after a failure tries again. Authentication is noninteractive, so password
login and passphrase-protected keys that are not loaded into an SSH agent are not
supported. A host seen for the first time is trusted and recorded in the SSH known
hosts file; a host whose recorded key changed is refused. Python 3 must be available on
the Linux x86_64 target. The extension asks to start with the application so saved
hosts are listed before any command runs.

The Agent source travels over SSH and is checked against its bundled SHA-256
before execution; no persistent Agent installation is required. Configuration
stores connection values only. This package never stores passwords or private
keys.

The search executable is compiled from plugin-owned Rust source and shares pure
text decoding/matching rules with the editor. The component transfers its gzip
bundle in 128 KiB chunks, checks the expanded SHA-256 and seals an anonymous
Linux file before execution. It does not install a target Rust compiler or a
WebAssembly runtime. Directory descriptors and no-follow file opens keep disk
reads inside the mounted folder; links and special files are skipped.

Search supports literal/regular expressions, case/word options, include/exclude
globs, hidden files, and scoped `.gitignore`, `.ignore`, `.rgignore` and root
`.git/info/exclude` rules. Ignore files outside the mounted root are not read.
Target filenames preserve literal backslashes. Unsaved text stays in Loog: the
scanner reports eligible buffer identities and the application merges matches
using the same text rules. Limits are 4 MiB per decoded file, 10,000 matches,
5,000 matching files, depth 64, one million visited entries, 1 GiB read bytes,
16 MiB output and 120 seconds per target scan. Ignore rules have a cumulative
1 MiB / 8,192-line budget; worker address space is capped at 256 MiB. Four
searches, including cancelled children awaiting collection, may run per
connection; cancellation closes their processes and unused handles expire.
The standalone Linux scanner passes encoding, ignore, bounds and cancellation
tests, including 100 cancellation cycles with children reaped and file
descriptors returning to baseline. Native search/navigation also passes the
complete headless SSH test, including dirty buffers, UTF-16, cancellation and
two environments sharing the same target paths.

Current development support includes directory pages, native text editing,
encoding/BOM preservation, revision-checked atomic replacement, and staged saves
up to 64 MiB. A revision check followed by replacement does not provide strict
compare-and-swap against arbitrary external writers. Conflicts and lost results
retain the editor contents. Use the tab's conflict actions to read the current
file before keeping your edits or reloading it.

Native text editing reads EditorConfig from the target ancestors, using the
provider's relative names even on Windows clients. Configuration changes,
deletion and recreation update open documents while preserving unsaved text,
undo history and manual editing choices. Reads have a fifteen-second total
deadline, 64 KiB per configuration and 512 KiB across at most 64 ancestors.

The native file tree supports creating files and directories, renaming them while
preserving open editor contents, and permanently deleting files or empty folders.
Deletion asks for the exact name; it does not use the local recycle bin.

Connected roots carry a provider-owned recovery description in the workspace
session. On restart, Loog activates this provider and the component reconnects
using existing SSH credentials. Cached unsaved text remains available offline.
The Disconnect command removes that root's recovery record; a connection failure
keeps it for retry. An absent or disabled provider leaves an offline root that can
be retried or removed. Up to four environments can connect independently. Run
Connect again to open another connection form; select an environment's SSH status
entry to inspect or disconnect it. The Disconnect command affects the selected
connection. Each SSH identity currently mounts one folder; use distinct SSH
aliases for separate environments. Changing a form never retargets an existing
connection. All process/timer IDs, resource scopes, replies and cancellation are
routed to their owning connection, including handles with equal numeric values.

Linux filesystem subscriptions are recursive and exclude directory symlinks.
Each subscription admits at most 4,096 directories and 65,536 entries at startup,
with 16 subscriptions per connection. Kernel overflow and directory moves trigger
a rescan. Losing coverage reports a paused subscription; Refresh retries it.
Events are hints, and file revisions are checked before saving. See the
[Linux inotify limitations](https://man7.org/linux/man-pages/man7/inotify.7.html),
including network filesystems and memory-mapped modifications. Native SSH tests
cover clean file refresh, conflict preservation, and directory creation/deletion.

The development Rust and JSON language plugins pass real SSH acceptance using
this provider's generic process and resource capabilities, including pinned
target tool preparation. Language behavior belongs to those plugins. Native
search and Source Control pass the same acceptance. Source Control discovers a
repository from the active target file and supports status, decoded text diffs,
stage/unstage and commit through target Git. Git must already be installed in the
target environment; this plugin does not install it or configure a commit identity.
Commit failures preserve the message. Disconnection keeps the selected repository
offline, and neither mutations nor terminal input are automatically replayed.
The resource panel currently shows one selected repository per mounted folder;
advanced history, branch/network operations, hunk patches and symlink text diffs
are outside its initial scope. Git metadata must stay within the mounted scope.
Native project `.loog/settings.json` and EditorConfig reads and monitoring pass
headless SSH acceptance, including change/delete/recreation, encoding and editing
precedence, provider-relative file associations and a project-selected Bash.
The existing Open Project Settings command creates a template in the connected
workspace, preserves existing files and allows malformed settings to be repaired.
Raster images and Markdown previews also use target bytes; relative images,
image refresh and document/heading/directory links pass the same native test.
Literal backslashes and Chinese names remain target filenames on a Windows client.
This provider requires UTF-8 filenames. A directory page containing an unsupported
byte filename reports an explicit error; it never substitutes a lossy name or
reports the directory as empty. Native local paths keep their existing OS behavior.
The package has not been published; its development version does not imply that
these features are complete.

The provider's process and PTY transport passes real component/SSH tests for
binary pipes, input credits, cancellation, Unicode terminal input, resize and
exit. Native terminal transport also passes headless application acceptance for
Unicode, resize, exit and trust revocation. Open in Terminal is available on
provider directories; workspace trust can be reviewed from the same menu.
Execution requires a separate workspace authorization, recorded by exact
provider/environment/root identity. Revocation keeps file access and monitoring.
Native terminal input uses a bounded queue and is discarded on failure or stop.
Restored terminal tabs stay stopped; New and Split create fresh provider sessions.
The Agent reports its terminal process cwd at most twice per second; New, Split
and saved terminal targets follow that provider-normalized directory. An unknown
or out-of-workspace cwd is shown as unavailable, and new sessions use the root.
Actual desktop visual verification is pending; no desktop automation is running.

The component renews an Agent lease every 10 seconds. Without incoming requests
for 45 seconds, the Agent closes staged writes, subscriptions and child process
sessions. Nonblocking protocol output is capped at 4 MiB; process input windows
are 256 KiB and output chunks 16 KiB. Cleanup covers process groups in the created
session, including terminal foreground jobs. Programs deliberately detaching into
another session, an uninterruptible kernel operation, SIGKILL of the Agent or
machine loss are outside that cleanup guarantee. Shell sessions are not replayed
or restored after an Agent restart.
