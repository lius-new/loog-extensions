# Vue Language

Syntax highlighting for Vue.

- Grammars: `vue`
- File types: `vue`
- License of the bundled grammar and queries: MIT

The grammar is a Tree-sitter parser compiled to WebAssembly from pinned sources;
`PROVENANCE.md` lists them and `licenses/` carries their licenses.

## Change how it is colored

Put your own `vue/highlights.scm` in Loom's queries folder. Start the file with
`; extends` to add rules to the ones here; without it your file replaces them. Use
**Inspect Highlight at Cursor** to see which rule colors a place.
