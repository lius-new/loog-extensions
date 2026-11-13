umask 077;
echo "loog-arch $(uname -m)";
IFS=" " read -r n h;
[ "$n" -gt 0 ] 2>/dev/null && [ "$n" -le 33554432 ] || { echo "loog-error: invalid agent size" >&2; exit 2; };
f=;
for c in "$XDG_RUNTIME_DIR" "$HOME/.cache" "${TMPDIR:-/tmp}" /tmp /dev/shm; do
  [ -n "$c" ] && [ -d "$c" ] && [ -w "$c" ] || continue;
  t="$c/loog-agent-$$";
  mkdir -m 700 "$t" 2>/dev/null || continue;
  if [ -z "$f" ]; then
    head -c "$n" > "$t/a" || { echo "loog-error: cannot store the agent in $c" >&2; rm -rf "$t"; exit 3; };
    if command -v sha256sum >/dev/null 2>&1; then
      [ "$(sha256sum < "$t/a" | cut -d " " -f 1)" = "$h" ] || { echo "loog-error: the agent was damaged in transit" >&2; rm -rf "$t"; exit 3; };
    fi;
    f="$t/a";
  else
    cp "$f" "$t/a" 2>/dev/null || { rm -rf "$t"; continue; };
  fi;
  chmod 700 "$t/a";
  if "$t/a" --probe 2>/dev/null; then exec "$t/a" --ephemeral "$t"; fi;
  [ "$t/a" = "$f" ] || rm -rf "$t";
done;
[ -n "$f" ] && rm -rf "${f%/a}";
echo "loog-error: no directory on this machine allows running the agent" >&2;
exit 4
