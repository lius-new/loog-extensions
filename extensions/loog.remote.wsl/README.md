# Remote WSL (development)

Open a folder of a Windows Subsystem for Linux distribution. The component owns
the connection, the connection UI and the bundled Linux Python Agent, and shares
its source with the SSH and container packages. Loog supplies generic window
commands, user tools and asynchronous resource operations; it does not recognize
WSL or this package's ID.

Build with LOOG_SOURCE pointing to the matching Loog checkout. Run `node scripts/sdk.mjs`
and build the search worker as described for Remote SSH, then `npm ci` and
`node build.mjs` in this extension folder.

Everything happens in the command palette. **Remote WSL: Connect to Distribution…**
lists the distributions that are installed, with their state and WSL version;
choose one to open its login folder. A distribution that connected is saved: type
`wsl` to list the saved ones and connect with Enter. The palette shows a turning
mark while connecting and a cross when it failed, with **Retry** to try again.
Distributions of Docker Desktop are not listed. A stopped distribution is started
by the connection.

Python 3 must be available in the distribution, which must be Linux x86_64. The
Agent source travels over standard input and is checked against its bundled
SHA-256 before execution; nothing is installed in the distribution. The search
executable is the one described for Remote SSH. The status bar shows the
distribution at the start of its right side while connected.

The package runs `wsl.exe`, so it works on Windows only. Nothing is stored but the
names of the distributions that connected.
