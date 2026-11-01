# Scala Language

Syntax highlighting for Scala.

- Grammars: `scala`
- File types: `scala`, `sc`
- License of the bundled grammar and queries: MIT

The grammar is a Tree-sitter parser compiled to WebAssembly from pinned sources;
`PROVENANCE.md` lists them and `licenses/` carries their licenses.

## Change how it is colored

Put your own `scala/highlights.scm` in Loom's queries folder. Start the file with
`; extends` to add rules to the ones here; without it your file replaces them. Use
**Inspect Highlight at Cursor** to see which rule colors a place.
