# Changelog

## Unreleased

- Reduced memory: the plugin no longer starts with the application. It starts on its
  first command, when a saved host is chosen in the palette, or when a remote
  workspace is restored. The palette lists the saved hosts from the rows the plugin
  last asked the application to keep (`loog:palette/retained`). Requires the matching
  development Loog palette interface. The bundled Linux agents are stored compressed
  and inflated while they are sent, and large Agent replies are read without an
  intermediate copy.
- Added plugin-owned SSH workspace connections with native file editing,
  revision-aware saves, monitoring, recovery and terminals.
- Moved connection into the command palette: saved hosts are listed by name,
  new hosts take `user@host[:port]`, and the private key (Default Location or
  a chosen file) is selected in the same place. Requires the matching development
  Loog palette interface.
- A new host asks which folder to open: the palette becomes a path field starting
  at the login folder, with the last five folders of the host and a file tree
  from `/`. Added `Open Folder…` to change the folder of a connection. Requires
  the matching development Loog palette interface.
- Added independent connection routing and target process support for language
  plugins. Requires the matching development Loog workspace interface.
- Replaced the Python agent with a static Rust agent (Linux x86_64 and aarch64)
  with search built in. The target now needs only a POSIX shell: Python is no
  longer required, and nothing remains installed after the connection ends.

This development package has not been published. See README.md for verified
behavior and current limitations.
