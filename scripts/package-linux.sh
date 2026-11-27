#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="${1:-$REPO_ROOT/dist}"
mkdir -p "$DIST_DIR"

VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$REPO_ROOT/Cargo.toml" | head -n 1)"
if [[ -z "$VERSION" ]]; then
    echo "Error: could not read version from Cargo.toml" >&2
    exit 1
fi

UNAME_ARCH="$(uname -m)"
case "$UNAME_ARCH" in
    x86_64)
        DEB_ARCH="amd64"
        RPM_ARCH="x86_64"
        ;;
    aarch64|arm64)
        DEB_ARCH="arm64"
        RPM_ARCH="aarch64"
        ;;
    *)
        echo "Unsupported architecture: $UNAME_ARCH" >&2
        exit 1
        ;;
esac

BINARY="$REPO_ROOT/target/release/blabber-llm"
if [[ ! -x "$BINARY" ]]; then
    echo "Error: release binary not found at $BINARY (run 'cargo build --release' first)" >&2
    exit 1
fi

DESKTOP_FILE="$REPO_ROOT/packaging/org.blabberllm.BlabberLLM.desktop"
LICENSE_FILE="$REPO_ROOT/LICENSE"

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# --- Build .deb package ---
DEB_STAGE="$WORK_DIR/deb-stage"
mkdir -p \
    "$DEB_STAGE/DEBIAN" \
    "$DEB_STAGE/usr/bin" \
    "$DEB_STAGE/usr/share/applications" \
    "$DEB_STAGE/usr/share/doc/blabber-llm"

install -m 0755 "$BINARY" "$DEB_STAGE/usr/bin/blabber-llm"
install -m 0644 "$DESKTOP_FILE" "$DEB_STAGE/usr/share/applications/org.blabberllm.BlabberLLM.desktop"
if [[ -f "$LICENSE_FILE" ]]; then
    install -m 0644 "$LICENSE_FILE" "$DEB_STAGE/usr/share/doc/blabber-llm/copyright"
fi

INSTALLED_SIZE_KB="$(du -sk "$DEB_STAGE/usr" | awk '{print $1}')"

cat > "$DEB_STAGE/DEBIAN/control" <<EOF
Package: blabber-llm
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Installed-Size: ${INSTALLED_SIZE_KB}
Depends: libgtk-4-1 (>= 4.16), libadwaita-1-0 (>= 1.6)
Recommends: pkexec
Maintainer: Jan Kammerath <https://github.com/jankammerath/BlabberLLM>
Homepage: https://github.com/jankammerath/BlabberLLM
Description: Native GTK4 and Libadwaita LLM chat GUI for GNOME
 BlabberLLM is an open GTK4 LLM chat client for GNOME supporting Claude,
 Gemini, Grok, OpenAI, OpenRouter, and local Ollama models.
EOF

DEB_OUT="$DIST_DIR/blabber-llm-${DEB_ARCH}.deb"
dpkg-deb --build --root-owner-group "$DEB_STAGE" "$DEB_OUT"
echo "Built $DEB_OUT"

# --- Build .rpm package ---
RPM_TOPDIR="$WORK_DIR/rpmbuild"
mkdir -p "$RPM_TOPDIR"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

RPM_VERSION="${VERSION//-/_}"
SPEC_FILE="$RPM_TOPDIR/SPECS/blabber-llm.spec"

cat > "$SPEC_FILE" <<EOF
Name:           blabber-llm
Version:        ${RPM_VERSION}
Release:        1
Summary:        Native GTK4 and Libadwaita LLM chat GUI for GNOME
License:        GPL-3.0-only
URL:            https://github.com/jankammerath/BlabberLLM
Requires:       gtk4 >= 4.16
Requires:       libadwaita >= 1.6
Recommends:     polkit

%define _build_id_links none

%description
BlabberLLM is an open GTK4 LLM chat client for GNOME supporting Claude,
Gemini, Grok, OpenAI, OpenRouter, and local Ollama models.

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/share/applications
mkdir -p %{buildroot}/usr/share/licenses/blabber-llm
install -m 0755 ${BINARY} %{buildroot}/usr/bin/blabber-llm
install -m 0644 ${DESKTOP_FILE} %{buildroot}/usr/share/applications/org.blabberllm.BlabberLLM.desktop
if [ -f "${LICENSE_FILE}" ]; then
    install -m 0644 ${LICENSE_FILE} %{buildroot}/usr/share/licenses/blabber-llm/LICENSE
fi

%files
/usr/bin/blabber-llm
/usr/share/applications/org.blabberllm.BlabberLLM.desktop
%license /usr/share/licenses/blabber-llm/LICENSE
EOF

rpmbuild \
    --define "_topdir $RPM_TOPDIR" \
    --target "$RPM_ARCH" \
    -bb "$SPEC_FILE"

RPM_BUILT="$(find "$RPM_TOPDIR/RPMS" -name '*.rpm' | head -n 1)"
RPM_OUT="$DIST_DIR/blabber-llm-${RPM_ARCH}.rpm"
cp "$RPM_BUILT" "$RPM_OUT"
echo "Built $RPM_OUT"
