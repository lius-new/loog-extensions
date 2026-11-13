# Changelog

## Unreleased

- Added plugin-owned SSH workspace connections with native file editing,
  revision-aware saves, monitoring, recovery and terminals.
- Moved connection into the command palette: saved hosts are listed by name,
  new hosts take `user@host[:port]`, and the private key (Default Location or
  a chosen file) is selected in the same place. Requires the matching development
  Loog palette interface.
- Added independent connection routing and target process support for language
  plugins. Requires the matching development Loog workspace interface.
- Replaced the Python agent with a static Rust agent (Linux x86_64 and aarch64)
  with search built in. The target now needs only a POSIX shell: Python is no
  longer required, and nothing remains installed after the connection ends.

This development package has not been published. See README.md for verified
behavior and current limitations.
