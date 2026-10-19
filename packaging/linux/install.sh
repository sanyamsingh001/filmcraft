#!/bin/sh
# Install the extracted Linux tarball. Defaults to ~/.local; use --prefix for another directory.
set -eu

install_prefix=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then
        printf '%s\n' 'error: --prefix needs an absolute directory' >&2
        exit 2
      fi
      install_prefix=$2
      shift 2
      ;;
    -h|--help)
      printf '%s\n' 'Usage: ./install.sh [--prefix /absolute/directory]' 'Default: ~/.local (no administrator privileges needed).'
      exit 0
      ;;
    *) printf 'error: unknown argument: %s\n' "$1" >&2; exit 2 ;;
  esac
done
if [ -z "$install_prefix" ]; then
  if [ -z "${HOME:-}" ]; then
    printf '%s\n' 'error: HOME is unset; use --prefix to choose an installation directory' >&2
    exit 2
  fi
  install_prefix="$HOME/.local"
fi
case "$install_prefix" in
  /*) ;;
  *) printf '%s\n' 'error: --prefix must be an absolute directory' >&2; exit 2 ;;
esac
app_here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ ! -f "$app_here/bin/filmcraft" ] || [ ! -f "$app_here/bin/filmcraft-cli" ] \
  || [ ! -f "$app_here/share/applications/ai.storyteller.filmcraft.desktop" ] \
  || [ ! -f "$app_here/share/icons/hicolor/256x256/apps/ai.storyteller.filmcraft.png" ] \
  || [ ! -f "$app_here/share/mime/packages/ai.storyteller.filmcraft.xml" ]; then
  printf '%s\n' 'error: keep install.sh beside the extracted bin/ and share/ directories' >&2
  exit 1
fi

mkdir -p "$install_prefix/bin" "$install_prefix/share"
install -m755 "$app_here/bin/filmcraft" "$install_prefix/bin/filmcraft"
install -m755 "$app_here/bin/filmcraft-cli" "$install_prefix/bin/filmcraft-cli"
cp -R "$app_here/share/." "$install_prefix/share/"

# GUI sessions need not include this prefix in PATH or their icon search directories.
# Exec has two escaping layers: Desktop Entry strings, then quoted command arguments.
# env also lets GLib validate entries whose binary path includes a literal percent sign.
app_id=ai.storyteller.filmcraft
FILMCRAFT_INSTALL_BINARY="$install_prefix/bin/filmcraft" \
FILMCRAFT_INSTALL_ICON="$install_prefix/share/icons/hicolor/256x256/apps/$app_id.png" \
awk '
  function string_value(s, out, i, c) {
    for (i = 1; i <= length(s); i++) {
      c = substr(s, i, 1)
      if (c == "\\") out = out "\\\\"
      else if (c == "\n") out = out "\\n"
      else if (c == "\r") out = out "\\r"
      else if (c == "\t") out = out "\\t"
      else out = out c
    }
    return out
  }
  function exec_value(s, out, i, c) {
    out = "\""
    for (i = 1; i <= length(s); i++) {
      c = substr(s, i, 1)
      if (c == "\\") out = out "\\\\\\\\"
      else if (c == "\"" || c == "`" || c == "$") out = out "\\\\" c
      else if (c == "%") out = out "%%"
      else out = out string_value(c)
    }
    return out "\""
  }
  /^Exec=/ { print "Exec=/usr/bin/env " exec_value(ENVIRON["FILMCRAFT_INSTALL_BINARY"]) " %F"; next }
  /^TryExec=/ { print "TryExec=" string_value(ENVIRON["FILMCRAFT_INSTALL_BINARY"]); next }
  /^Icon=/ { print "Icon=" string_value(ENVIRON["FILMCRAFT_INSTALL_ICON"]); next }
  { print }
' "$app_here/share/applications/$app_id.desktop" > "$install_prefix/share/applications/$app_id.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "$install_prefix/share/applications" || printf '%s\n' 'warning: desktop cache could not be refreshed' >&2
fi
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database "$install_prefix/share/mime" || printf '%s\n' 'warning: MIME cache could not be refreshed' >&2
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f "$install_prefix/share/icons/hicolor" || printf '%s\n' 'warning: icon cache could not be refreshed' >&2
fi
printf 'Installed FilmCraft in %s\n' "$install_prefix"
case ":${PATH:-}:" in
  *:"$install_prefix/bin":*) ;;
  *) printf 'Add %s/bin to PATH to run filmcraft from a terminal.\n' "$install_prefix" ;;
esac
