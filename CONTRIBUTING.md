# Contributing

## Compatibility code

New or modified code retained for an older API, format, behavior, environment,
or caller must have a local `COMPAT(<stable-reason>)` comment at each independent
compatibility branch or adapter. A note elsewhere in the repository is not a
substitute. Reuse the same reason identifier for related compatibility paths.

Each comment must identify:

- The older interface or behavior being supported.
- Why the compatibility code is still required.
- The replacement that new callers should use.
- The concrete condition for removing the compatibility code, including a
  removal version or date only when one has been agreed.

If no removal version is planned, explicitly say so; for example: `No removal
version has been scheduled.` This does not replace the removal condition. Compatibility
support alone does not mean an API is deprecated: do not claim imminent removal
or add deprecation attributes without an agreed deprecation plan.

For example:

```text
// COMPAT(legacy-widget-api):
// Supports: widgets created through the previous initialization API.
// Reason: existing callers have not all migrated.
// Replacement: use the current initialization API.
// Removal: remove after support for the previous API ends.
// No removal version has been scheduled.
```

When changing an existing compatibility path, reassess its removal condition.
If it is met, remove the compatibility code or explain why it must remain.
When removing compatibility code, update its compatibility-only tests,
documentation, and related markers in the same change. Preserve tests for
behavior that remains supported.

Ownership checks, lifecycle validation, and recovery required by the current
contract are not automatically compatibility code. Reviewers must check for
missing annotations as well as incomplete ones; a marker format check alone
cannot reliably detect an unannotated compatibility path.
