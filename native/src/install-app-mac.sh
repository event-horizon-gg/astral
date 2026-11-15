#!/bin/sh
set -eu
parent=$1
target=$2
stage=$3
ack=$4
token=$5
previous="$stage/previous.app"
next="$stage/Astral Native.app"
executable="Contents/MacOS/astral-native"
exec >>"$stage/install.log" 2>&1

attempt=0
while kill -0 "$parent" 2>/dev/null; do
  attempt=$((attempt + 1))
  [ "$attempt" -lt 60 ] || exit 1
  /bin/sleep 1
done
# Fail closed if process enumeration itself failed.
if /usr/bin/pgrep -x 'Wow|World of Warcraft|WowClassic|WowF|WowForever' >/dev/null; then
  "$target/$executable" --astral-update-recovery wow-running &
  exit 1
else
  if [ "$?" -ne 1 ]; then
    "$target/$executable" --astral-update-recovery game-check-failed &
    exit 1
  fi
fi

if [ -e "$ack" ] || [ -L "$ack" ]; then
  "$target/$executable" --astral-update-recovery replacement-failed &
  exit 1
fi
if ! /bin/mv "$target" "$previous"; then
  "$target/$executable" --astral-update-recovery replacement-failed &
  exit 1
fi
if ! /bin/mv "$next" "$target"; then
  /bin/mv "$previous" "$target"
  "$target/$executable" --astral-update-recovery replacement-failed &
  exit 1
fi
"$target/$executable" --astral-update-ack "$token" &
child=$!
attempt=0
while [ "$attempt" -lt 60 ]; do
  [ ! -f "$ack" ] || break
  kill -0 "$child" 2>/dev/null || break
  attempt=$((attempt + 1))
  /bin/sleep 1
done
if [ -f "$ack" ]; then
  /bin/rm -f "$ack"
  /bin/rm -rf "$stage"
  exit 0
fi
kill "$child" 2>/dev/null || true
attempt=0
while kill -0 "$child" 2>/dev/null; do
  attempt=$((attempt + 1))
  [ "$attempt" -lt 15 ] || exit 1
  /bin/sleep 1
done
/bin/mv "$target" "$stage/failed.app"
/bin/mv "$previous" "$target"
/bin/rm -f "$ack" || true
"$target/$executable" --astral-update-recovery rolled-back &
