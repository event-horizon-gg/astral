#!/bin/sh
set -eu
required=false
if [ "${1:-}" = --persistent ]; then
  required=true
  shift
fi
[ "$#" -le 1 ] || { echo 'Unexpected signing arguments.' >&2; exit 1; }
config=${1:-"$(dirname "$0")/../.astral-mac-certificate"}
identity=${ASTRAL_MAC_SIGN_IDENTITY:-}
without_controls=$(printf '%s' "$identity" | LC_ALL=C tr -d '[:cntrl:]')
if [ "$identity" != "$without_controls" ]; then
  echo 'Invalid macOS signing identity.' >&2
  exit 1
fi
identity=$(printf '%s' "$identity" | sed 's/^ *//;s/ *$//')
if [ -z "$identity" ]; then
  if [ -e "$config" ]; then
    identity=$(cat "$config")
    case "$identity" in
      ''|*[!0-9a-fA-F]*) echo 'Invalid .astral-mac-certificate: expected a certificate fingerprint.' >&2; exit 1 ;;
    esac
    [ "${#identity}" -eq 40 ] || { echo 'Invalid certificate fingerprint.' >&2; exit 1; }
  else
    identity=-
  fi
fi
case "$identity" in
  -) ;;
  -*) echo 'Invalid macOS signing identity.' >&2; exit 1 ;;
esac
if [ "$required" = true ]; then
  case "$identity" in
    *[!0-9a-fA-F]*) echo 'Configure a persistent macOS signing certificate before installing.' >&2; exit 1 ;;
  esac
  [ "${#identity}" -eq 40 ] || { echo 'A persistent certificate fingerprint is required.' >&2; exit 1; }
fi
printf '%s\n' "$identity"
