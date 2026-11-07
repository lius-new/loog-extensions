# Bash Language

Syntax highlighting for Bash.

- Grammar: `bash`
- File types: `sh`, `bash`, `.bashrc`, `.bash_profile`
- License of the bundled grammar and queries: MIT

The grammar is the upstream Tree-sitter parser compiled to WebAssembly from a pinned
source; `PROVENANCE.md` names it and `LICENSE` is its license.

## Change how it is colored

Put your own `bash/highlights.scm` in Loog's queries folder. Start the file with
`; extends` to add rules to the ones here; without it your file replaces them. Use
**Inspect Highlight at Cursor** to see which rule colors a place.
