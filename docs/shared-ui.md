# Plugin-owned shared status UI

## Required boundary

Plugins MUST use Loog's general capabilities. They MUST NOT require an
application-owned adapter, container declaration, plugin-ID check, server-name
check, state mapping or shared-key registration in Loog. A new plugin using the
existing shared UI contract does not require an application release.

The application owns the generic `loog:ui/shared@0.1.0` mechanism and private node
ownership. Plugins own the chosen key, candidate trees, eligible contexts,
priorities, state interpretation, text, colors, icons and keyframes. No plugin
may read or mutate another plugin's raw UI handles. Joining a shared region is
an opt-in contribution, not cross-plugin write access.

## The three language clients

The independently compiled Rust clients share private source in
`language-clients/rust`. Their status policy uses
`loog.language-services.status.v1`; this constant exists in plugin code only.
Each open project/session contributes a private, passive tree to that key, with
the exact `DocumentRef.path` values it currently serves as eligible contexts.
The native statusbar mounts one shared region for the active document, not one
block per language server or a set of hidden duplicate native widgets.

The common source supplies the following policy; Loog treats the numbers and
strings as opaque input:

| Plugin state | Priority | Presentation |
| --- | ---: | --- |
| Error | 3 | Red alert circle, `Language Services: Error` |
| Starting/working | 2 | Purple continuous spinner, generic Starting/Working text |
| Ready | 1 | Green check circle, `Language Services: Ready` |
| Stopped | 0 | Gray outlined stop circle, `Language Services: Stopped` |

The row has no independent background. Its text is neutral gray; the icon alone
uses the state color. Specific server names, project names, percentages and error
reasons are absent from this entry. Detailed failures remain in plugin logs.
The entry stays passive: no new click, hover or details UI is introduced here.
There is no trailing spacer: the native statusbar supplies the single 10px gap
to its next item. Generic text paint slack does not increase the layout width.

Each session keeps its own state and document membership. A failed service in a
different document/project cannot override the current document's entry.
Priority only orders overlapping eligible contributions. Tie-breaking is the
host's general stable owner/local-ID ordering, not last-writer-wins.

Only a changed state starts/stops motion or updates the icon. Progress does not
restart rotation, republish the contribution, or include changing percentages
in the shared text. A generic Starting-to-Working text change may update the text
child while the same busy animation continues. Identical text does nothing.
Ready, stopped and error states are static.

The four status icons use Material Design Icons: `Loading`, `CheckCircle`,
`AlertCircle` and `StopCircleOutline` (Pictogrammers, Apache-2.0). Their path data
is embedded in this repository's private Rust client and sent with the generic
`svg-path:24x24:` icon syntax; no icon aliases or paths are added to Loog's
registry. Local acceptance requires a host supporting that syntax. Retain
14px icon boxes, 900ms linear busy rotation and the surrounding layout spacing.

Closing a session's last document removes that session's candidate. Unloading
one plugin releases only its contributions; another eligible plugin keeps the
region available. No first creator owns the shared region's lifetime. Reloaded
instances use new owner generations, and stale events cannot regain ownership.
No applicable contribution hides the entry; closing it does not stop a server
that is still serving other documents.

## Build and acceptance constraints

Use `LOOG_SOURCE` pointing to the matching Loog checkout. The builder rejects
SDKs missing tree, motion or shared imports. Each plugin still has its own Cargo
manifest, lock file, component, process, capabilities and lifecycle. Sharing
source or a native UI region does not merge their WASM instances or authority.

Keep publications within the host's existing UI/node/round budgets and shared
context limits (256 contexts, 32 KiB total, 4 KiB each). Errors must be surfaced
in plugin logs rather than silently truncating membership. New shared-key or
policy versions belong in this repository, not in application source.

Local test packages may be rebuilt/staged without changing manifest versions.
Do not publish or alter immutable release packages during development. A future
release must first align the committed SDK source, importer/validator and AOT
compiler pins with the shared interface, then run all three component checks.
