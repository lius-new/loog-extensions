; COMPAT(typst-injection-shebang): Loog 0.2.1 rejects the upstream shebang
; capture. Keep language-tagged code blocks working with the supported captures.
; Use the upstream query once the minimum Loog version supports shebangs.
; No removal version is scheduled.
(raw_blck
  lang: (ident) @injection.language
  (blob) @injection.content)
