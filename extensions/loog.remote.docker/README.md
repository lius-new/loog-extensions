# Remote Containers (development)

Open a folder inside a running Docker container. The component owns the
connection, the connection UI and the bundled static Linux agents, and shares its
source with the SSH and WSL packages. Loog supplies generic window commands, user
tools and asynchronous resource operations; it does not recognize Docker or this
package's ID.

Build with LOOG_SOURCE pointing to the matching Loog checkout. Run `node scripts/sdk.mjs`,
add the targets (`rustup target add wasm32-unknown-unknown x86_64-unknown-linux-musl aarch64-unknown-linux-musl`),
then `npm ci` and `node build.mjs` in this extension folder; it builds both agents first.

Everything happens in the command palette. **Remote Containers: Connect to
Container…** lists the running containers with their image and status; choose one
to open the folder the container starts in. A container that connected is saved:
type `container` to list the saved ones and connect with Enter. The palette shows
a turning mark while connecting and a cross when it failed, with **Retry** to try
again.

The `docker` command must be installed and able to reach a Docker engine. The
container needs nothing but a POSIX shell (`sh`) and may be Linux x86_64 or
aarch64. The agent travels over standard input, is checked against its bundled
SHA-256 and runs from a private temporary folder that it removes as it starts;
nothing is installed in the container. Only running containers are
listed. The status bar shows the container at the start of its right side while
connected.
