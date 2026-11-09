#!/bin/sh
# Download, verify, and install without gh or sudo. curl ... | sh
# Environment: VERSION, PREFIX, VORK_REPO, VORK_DOWNLOAD_BASE; VORK_SKIP_SETUP=1 skips `vork setup` (GPU-free CI);
# VORK_NO_MODIFY_PATH=1 leaves shell startup files alone. A custom PREFIX never edits them.
set -eu
repo=${VORK_REPO:-voyager-so/vork}
# Default version is generated from Cargo.toml by scripts/sync-version.
version=${VERSION:-0.3.0}
default_prefix=$HOME/.local
prefix=${PREFIX:-"$default_prefix"}
# POSIX shell literals for copy/paste, including embedded quotes and $/backticks.
shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}
dry_run=false
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) dry_run=true; shift ;;
        --help) echo 'Usage: install.sh [--dry-run]'; exit 0 ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done
case "$version" in *[!A-Za-z0-9._-]*|'') echo 'Invalid VERSION' >&2; exit 2;; esac
case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
        platform=darwin-arm64
        macos=$(sw_vers -productVersion 2>/dev/null || true)
        if ! awk -v v="$macos" 'BEGIN {split(v,a,"."); exit !(a[1] ~ /^[0-9]+$/ && a[1]>=26)}'; then
            echo "Requires Apple Silicon macOS 26+ because the bundled Slang compiler and libraries target macOS 26; found ${macos:-unknown}. Nothing was downloaded." >&2
            exit 1
        fi ;;
    Linux-x86_64)
        platform=linux-x64
        glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
        if ! awk -v v="$glibc" 'BEGIN {split(v,a,"."); exit !(a[1]>2 || (a[1]==2 && a[2]>=35))}'; then
            echo "Requires glibc 2.35+; found ${glibc:-unknown}" >&2; exit 1
        fi ;;
    *) echo 'Supported platforms: Apple Silicon macOS and Linux x64' >&2; exit 1 ;;
esac
archive="vork-${version}-${platform}.tar.gz"
base=${VORK_DOWNLOAD_BASE:-"https://github.com/$repo/releases/download/v$version"}
base=${base%/}
# A downloaded installer can accompany an archive; piped installs still download.
installer_dir=
if [ -f "$0" ]; then installer_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P); fi
if "$dry_run"; then
    source_base=$base
    if [ -n "$installer_dir" ] && [ -f "$installer_dir/$archive" ]; then source_base=$installer_dir; fi
    printf 'Archive: %s/%s\nChecksums: %s/SHA256SUMS\nBinary: %s/bin/vork\n' "$source_base" "$archive" "$source_base" "$prefix"
    exit 0
fi
command -v curl >/dev/null 2>&1 || { echo 'vork install needs curl; install it and run this again.' >&2; exit 1; }
work=$(mktemp -d)
stage=''
retired=''
cleanup() {
    rm -rf "$work"
    if [ -n "$stage" ]; then rm -rf "$stage"; fi
    if [ -n "$retired" ]; then
        if [ -e "$retired/vork" ] && [ ! -e "$prefix/lib/vork" ]; then
            if ! mv "$retired/vork" "$prefix/lib/vork"; then
                echo "Could not restore the previous install; it is preserved at $retired/vork" >&2
                return
            fi
        fi
        rm -rf "$retired"
    fi
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM
fetch() {
    curl -fL --connect-timeout 30 --speed-limit 1024 --speed-time 120 --retry 3 "$base/$1" -o "$work/$1" || {
        echo "Could not download $base/$1" >&2
        echo "Nothing was installed. Check free space in $work, your connection (downloads stop after 120 seconds below 1 KiB/s), and that vork $version has this file: https://github.com/$repo/releases" >&2
        exit 1
    }
}
if [ -n "$installer_dir" ] && [ -f "$installer_dir/$archive" ]; then
    echo "Using bundled archive: $installer_dir/$archive"
    [ -f "$installer_dir/SHA256SUMS" ] || { echo 'Bundled archive requires SHA256SUMS beside install.sh' >&2; exit 1; }
    cp "$installer_dir/$archive" "$work/$archive"
    cp "$installer_dir/SHA256SUMS" "$work/SHA256SUMS"
else
    fetch "$archive"
    fetch SHA256SUMS
fi
(
    cd "$work" || exit 1
    awk -v name="$archive" '$2 == name || $2 == "*" name {print}' SHA256SUMS > selected.sha256
    [ "$(wc -l < selected.sha256 | tr -d ' ')" = 1 ] || { echo "Missing or ambiguous checksum for $archive; nothing was installed." >&2; exit 1; }
    if command -v sha256sum >/dev/null 2>&1; then sha256sum -c selected.sha256 || exit 2
    else shasum -a 256 -c selected.sha256 || exit 2; fi
) || {
    status=$?
    if [ "$status" = 2 ]; then echo "Checksum mismatch for $archive: the download is damaged or altered. Nothing was installed; run this again." >&2; fi
    exit 1
}
# Refuse traversal paths before extracting a verified archive.
if tar -tzf "$work/$archive" | LC_ALL=C grep -E '(^/|(^|/)\.\.(/|$))' >/dev/null; then
    echo 'Unsafe archive path' >&2; exit 1
fi
mkdir -p "$prefix/bin" "$prefix/lib"
prefix=$(cd "$prefix" && pwd -P)
# Build the new tree beside the old one and swap it in whole: files a release dropped
# do not survive an upgrade, and a failed install leaves the old one working.
stage=$(mktemp -d "$prefix/lib/.vork-new.XXXXXX")
if ! tar -xzf "$work/$archive" -C "$stage" 2>"$work/extract.log"; then
    echo "Could not extract vork into $stage; the existing install was not changed." >&2
    echo "Check free space and write permissions on $prefix (the expanded release must fit beside the existing install). The downloaded archive also occupies space in $work." >&2
    head -n 8 "$work/extract.log" >&2
    exit 1
fi
chmod 755 "$stage"
if "$stage/vork" --version >"$work/version.log" 2>&1; then :
else
    status=$?
    if [ "$status" = 126 ]; then
        echo "Cannot execute $stage/vork. Check executable permissions and whether $prefix is mounted noexec; choose an executable filesystem with PREFIX. The existing install was not changed." >&2
    else
        echo "This vork build could not start (exit $status); check the native error below for missing runtime libraries or platform incompatibility. The existing install was not changed." >&2
    fi
    cat "$work/version.log" >&2
    exit 1
fi
if [ -e "$prefix/lib/vork" ]; then
    retired=$(mktemp -d "$prefix/lib/.vork-old.XXXXXX")
    mv "$prefix/lib/vork" "$retired/vork"
fi
mv "$stage" "$prefix/lib/vork"
stage=''
ln -sf "$prefix/lib/vork/vork" "$prefix/bin/vork"
"$prefix/bin/vork" --version

# Let new shells find vork. Only the default location touches startup files.
quoted_exe=$(shell_quote "$prefix/bin/vork")
quoted_bin=$(shell_quote "$prefix/bin")
path_line="export PATH=$quoted_bin:\"\$PATH\""
path_note=''
if [ -n "${PREFIX:-}" ] || [ "${VORK_NO_MODIFY_PATH:-}" = 1 ]; then
    case ":$PATH:" in
        *":$prefix/bin:"*) ;;
        *) path_note="Add vork to PATH by putting this in your shell startup file:
  $path_line" ;;
    esac
else
    path_line='export PATH="$HOME/.local/bin:$PATH"'
    case "${SHELL:-}" in
        # Every zsh reads .zshenv; .zshrc is interactive only.
        */zsh) rc_files="${ZDOTDIR:-$HOME}/.zshenv" ;;
        */bash)
            # Interactive shells read .bashrc. Login shells read only the first of these, and a stock
            # .bashrc returns early when not interactive, so the login file needs the line as well.
            if [ -e "$HOME/.bash_profile" ]; then login_rc="$HOME/.bash_profile"
            elif [ -e "$HOME/.bash_login" ]; then login_rc="$HOME/.bash_login"
            elif [ -e "$HOME/.profile" ] || [ "$(uname -s)" != Darwin ]; then login_rc="$HOME/.profile"
            else login_rc="$HOME/.bash_profile"; fi
            rc_files="$HOME/.bashrc
$login_rc" ;;
        */fish)
            rc_files="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/vork.fish"
            path_line='fish_add_path -g "$HOME/.local/bin"'
            mkdir -p "$(dirname "$rc_files")" 2>/dev/null || true ;;
        *) rc_files="$HOME/.profile" ;;
    esac
    added=''
    failed=''
    while IFS= read -r rc; do
        if [ -f "$rc" ] && grep -Fqx "$path_line" "$rc"; then added="$added $rc"; continue; fi
        if printf '\n# Added by the vork installer\n%s\n' "$path_line" >> "$rc" 2>/dev/null; then added="$added $rc"
        else failed="$failed $rc"; fi
    done <<RC_FILES
$rc_files
RC_FILES
    if [ -n "$added" ]; then path_note="New shells that read$added find vork. In this shell, or one that reads none of those (such as plain \`bash -c\`), run:
  $path_line
or call it directly: $quoted_exe"; fi
    if [ -n "$failed" ]; then path_note="${path_note:+$path_note
}Could not write$failed. Put this in your shell startup file:
  $path_line"; fi
fi

if [ "${VORK_SKIP_SETUP:-}" = 1 ]; then
    echo 'Skipping vork setup (VORK_SKIP_SETUP=1); run `vork setup` before rendering.'
else
    echo 'Preparing shaders for this GPU; native progress reports the current phase...'
    "$prefix/bin/vork" setup || {
        echo "vork is installed at $prefix/bin/vork, but setup failed. Follow the report above, then run: $quoted_exe setup" >&2
        if [ -n "$path_note" ]; then printf '%s\n' "$path_note" >&2; fi
        exit 1
    }
fi
echo "Installed to $prefix/bin/vork."
if [ -n "$path_note" ]; then printf '%s\n' "$path_note"; fi
