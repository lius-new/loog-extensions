# Remote Containers (development)

Open a folder inside a running Docker container. The component owns the
connection, the connection UI and the bundled Linux Python Agent, and shares its
source with the SSH and WSL packages. Loog supplies generic window commands, user
tools and asynchronous resource operations; it does not recognize Docker or this
package's ID.

Build with LOOG_SOURCE pointing to the matching Loog checkout. Run `node scripts/sdk.mjs`
and build the search worker as described for Remote SSH, then `npm ci` and
`node build.mjs` in this extension folder.

Everything happens in the command palette. **Remote Containers: Connect to
Container…** lists the running containers with their image and status; choose one
to open the folder the container starts in. A container that connected is saved:
type `container` to list the saved ones and connect with Enter. The palette shows
a turning mark while connecting and a cross when it failed, with **Retry** to try
again.

The `docker` command must be installed and able to reach a Docker engine. Python 3
must be available in the container, which must be Linux x86_64. The Agent source
travels over standard input and is checked against its bundled SHA-256 before
execution; nothing is installed in the container. Only running containers are
listed. The status bar shows the container at the start of its right side while
connected.
