# Remote WSL (development)

Open a folder of a Windows Subsystem for Linux distribution. The component owns
the connection, the connection UI and the bundled static Linux agents, and shares
its source with the SSH and container packages. Loog supplies generic window
commands, user tools and asynchronous resource operations; it does not recognize
WSL or this package's ID.

Build with LOOG_SOURCE pointing to the matching Loog checkout. Run `node scripts/sdk.mjs`,
add the targets (`rustup target add wasm32-unknown-unknown x86_64-unknown-linux-musl aarch64-unknown-linux-musl`),
then `npm ci` and `node build.mjs` in this extension folder; it builds both agents first.

Everything happens in the command palette. **Remote WSL: Connect to Distribution…**
lists the distributions that are installed, with their state and WSL version;
choose one to open its login folder. A distribution that connected is saved: type
`wsl` to list the saved ones and connect with Enter. The palette shows a turning
mark while connecting and a cross when it failed, with **Retry** to try again.
Distributions of Docker Desktop are not listed. A stopped distribution is started
by the connection.

The distribution needs nothing but a POSIX shell: no Python and no other runtime.
It may be Linux x86_64 or aarch64. The agent travels over standard input, is
checked against its bundled SHA-256 and runs from a private temporary folder that
it removes as it starts; nothing is installed in the distribution. Search runs
inside the same executable. The status bar shows the
distribution at the start of its right side while connected.

The package runs `wsl.exe`, so it works on Windows only. Nothing is stored but the
names of the distributions that connected.
