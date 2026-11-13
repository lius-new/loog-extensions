# Changelog

## Unreleased

- Added plugin-owned container workspace connections from the command palette,
  with the running containers listed by image and status, saved containers, a
  status bar entry and retry after a failure. Shares its Agent with Remote SSH.
  Requires the matching development Loog palette interface.
- A new container asks which folder to open from a path field and file tree;
  `Open Folder…` changes it later. Requires the matching development Loog palette
  interface.
- The container needs only a POSIX shell: the static Rust agent (Linux x86_64 and
  aarch64) replaces the Python agent, so Python is no longer required.

This development package has not been published. See README.md for current
behavior and limitations.
