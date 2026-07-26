#!/usr/bin/env bash
# deck-diag.sh — run Deck-side diagnostics for the Uplink Flatpak and write the
# result to deck-diag-output.txt NEXT TO THIS SCRIPT, so it syncs back over SMB
# for the dev to read (and it's also printed to your terminal).
#
# REUSABLE: the dev edits the commands in the DIAGNOSTICS block below, you re-run
#   bash deck-diag.sh
# and the output file updates. No need to hand-type individual commands.
set +e   # keep going even if a step fails

HERE="$(cd "$(dirname "$0")" && pwd)"
OUT="$HERE/deck-diag-output.txt"
# fall back to $HOME if the shared folder is mounted read-only
( : > "$OUT" ) 2>/dev/null || OUT="$HOME/deck-diag-output.txt"
ID="com.goblinengineering.uplink"

{
  echo "=== deck-diag @ $(date) ==="
  echo "host: $(uname -a)"

  # ────────────── DIAGNOSTICS (dev edits this block) ──────────────

  echo; echo "### A. current remotes (looking for a stale uplink/local one) ###"
  flatpak remotes --user

  echo; echo "### B. currently installed uplink refs ###"
  flatpak list --user --columns=application,branch,origin 2>&1 | grep -i uplink || echo "(none listed)"

  echo; echo "### C. PURGE the old install + any leftover uplink remotes ###"
  flatpak uninstall --user -y "$ID" 2>&1 | tail -3 || echo "(nothing to uninstall)"
  for r in $(flatpak remotes --user --columns=name 2>/dev/null | grep -iE "uplink|gec|local"); do
    echo "removing remote: $r"; flatpak remote-delete --user --force "$r" 2>&1
  done

  echo; echo "### D. install the CI bundle FRESH (full output) ###"
  B="$(ls -t "$HOME"/Downloads/gec-uplink*.flatpak 2>/dev/null | head -1)"
  echo "bundle file: [$B]  size: $(stat -c%s "$B" 2>/dev/null || echo '?') bytes"
  flatpak install --user -y --bundle "$B" 2>&1 | tail -20

  echo; echo "### E. verify it deployed + what ref/branch ###"
  flatpak list --user --columns=application,branch,arch,origin,installed-size 2>&1 | grep -i uplink || echo "(NOT installed)"
  flatpak info --user "$ID" 2>&1 | grep -iE "ref|branch|arch|version|installed" | head

  echo; echo "### F. launch it (backgrounded) + is it running? ###"
  ( flatpak run "$ID" >/tmp/uplink-run.log 2>&1 & )
  sleep 4
  echo "run log:"; cat /tmp/uplink-run.log 2>/dev/null | head -20
  echo "still running?:"; flatpak ps 2>&1 | grep -i uplink || echo "(not running — exited)"

  # ─────────────────────── end DIAGNOSTICS ────────────────────────

  echo; echo "=== done ==="
} 2>&1 | tee "$OUT"

echo
echo ">>> wrote: $OUT"
echo ">>> that file should sync back over SMB; if not, open it and paste its contents."
