# Changelog

## 0.1.1

- Requires Loog 0.2.310 or newer for the retained command palette entries and
  workspace interfaces used by this release.
- Reduced memory: the plugin no longer starts with the application. It starts on its
  first command, when a saved container is chosen in the palette, or when a remote
  workspace is restored. The palette lists the saved containers from the rows the plugin
  last asked the application to keep (`loog:palette/retained`). The bundled Linux
  agents are stored compressed and inflated while they are sent, and large Agent
  replies are read without an intermediate copy.

## 0.1.0

- Added plugin-owned container workspace connections from the command palette,
  with the running containers listed by image and status, saved containers, a
  status bar entry and retry after a failure. Shares its Agent with Remote SSH.
  Requires the matching development Loog palette interface.
- A new container asks which folder to open from a path field and file tree;
  `Open Folder…` changes it later. Requires the matching development Loog palette
  interface.
- The container needs only a POSIX shell: the static Rust agent (Linux x86_64 and
  aarch64) replaces the Python agent, so Python is no longer required.
