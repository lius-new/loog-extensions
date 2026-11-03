# Repository instructions

Follow the [compatibility code policy](CONTRIBUTING.md#compatibility-code).
Every new or modified compatibility branch or adapter needs a local
`COMPAT(<stable-reason>)` comment covering the supported legacy behavior,
retention reason, replacement, and removal condition. Explicitly state when
no removal version is scheduled; do not invent a deprecation plan. Reassess
existing compatibility paths when touching them, and update compatibility-only
tests, documentation, and markers when removing them. Apply this during code
reviews as well as implementation.
