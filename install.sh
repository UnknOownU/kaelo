#!/usr/bin/env sh
# Kaelo installer — downloads the correct binary from GitHub Releases.
# Usage: curl -fsSL https://raw.githubusercontent.com/HachemiH/kaelo/main/install.sh | sh
# Options:
#   --version vX.Y.Z   Install a specific version (default: latest)
#   --dir /path         Install to a custom directory (default: ~/.local/bin)

set -eu

REPO="HachemiH/kaelo"
INSTALL_DIR="${HOME}/.local/bin"
REQUESTED_VERSION=""

# --- Parse arguments ---
for arg in "$@"; do
  case "$arg" in
    --version=*)
      REQUESTED_VERSION="${arg#--version=}"
      ;;
    --version)
      # handled in next iteration via shift — but in pipe mode just skip
      ;;
    --dir=*)
      INSTALL_DIR="${arg#--dir=}"
      ;;
    --dir)
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

# --- Helpers ---
info()  { printf '[info]  %s\n' "$*"; }
warn()  { printf '[warn]  %s\n' "$*" >&2; }
error() { printf '[error] %s\n' "$*" >&2; exit 1; }

check_cmd() {
  command -v "$1" >/dev/null 2>&1 || error "'$1' is required but not found in PATH"
}

# --- Preflight checks ---
check_cmd curl
check_cmd shasum
check_cmd tar

# --- Detect OS and architecture ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "${OS}" in
  Darwin) OS="darwin" ;;
  Linux)  OS="linux" ;;
  *) error "Unsupported OS: ${OS}" ;;
esac

case "${ARCH}" in
  x86_64|amd64)   ARCH="x86_64" ;;
  aarch64|arm64)  ARCH="aarch64" ;;
  *) error "Unsupported architecture: ${ARCH}" ;;
esac

TARGET="${ARCH}-${OS}"

# Map to Rust target triple
case "${TARGET}" in
  aarch64-darwin)       RUST_TARGET="aarch64-apple-darwin" ;;
  x86_64-darwin)        RUST_TARGET="x86_64-apple-darwin" ;;
  x86_64-linux)         RUST_TARGET="x86_64-unknown-linux-gnu" ;;
  aarch64-linux)        RUST_TARGET="aarch64-unknown-linux-gnu" ;;
  *) error "No binary available for ${TARGET}" ;;
esac

# --- Resolve version ---
if [ -n "${REQUESTED_VERSION}" ]; then
  VERSION="${REQUESTED_VERSION}"
  info "Installing requested version: ${VERSION}"
else
  info "Fetching latest version from GitHub..."
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | head -1 \
    | sed -E 's/.*"([^"]+)".*/\1/') \
    || error "Failed to fetch latest version from GitHub"
  [ -z "${VERSION}" ] && error "Could not determine latest version"
  info "Latest version: ${VERSION}"
fi

# --- Download ---
ARCHIVE="kaelo-${VERSION}-${RUST_TARGET}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ARCHIVE}"
CHECKSUM_URL="https://github.com/${REPO}/releases/download/${VERSION}/${CHECKSUM}"

TMPDIR=$(mktemp -d)
trap 'rm -rf "${TMPDIR}"' EXIT

info "Downloading ${ARCHIVE}..."
curl -fsSL -o "${TMPDIR}/${ARCHIVE}" "${DOWNLOAD_URL}" \
  || error "Failed to download ${ARCHIVE}"

info "Downloading checksum..."
curl -fsSL -o "${TMPDIR}/${CHECKSUM}" "${CHECKSUM_URL}" \
  || error "Failed to download checksum"

# --- Verify checksum ---
info "Verifying SHA256 checksum..."
cd "${TMPDIR}"
EXPECTED=$(awk '{print $1}' "${CHECKSUM}")
ACTUAL=$(shasum -a 256 "${ARCHIVE}" | awk '{print $1}')

if [ "${EXPECTED}" != "${ACTUAL}" ]; then
  error "Checksum mismatch!\n  expected: ${EXPECTED}\n  actual:   ${ACTUAL}"
fi
info "Checksum verified."

# --- Install ---
mkdir -p "${INSTALL_DIR}"

info "Extracting kaelo to ${INSTALL_DIR}..."
tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}" kaelo
chmod +x "${TMPDIR}/kaelo"
cp "${TMPDIR}/kaelo" "${INSTALL_DIR}/kaelo"

# --- Done ---
INSTALLED="${INSTALL_DIR}/kaelo"
info "Installed kaelo ${VERSION} to ${INSTALLED}"

# Check if install dir is in PATH
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    # already in PATH
    ;;
  *)
    printf '\n'
    info "Add ${INSTALL_DIR} to your PATH:"
    printf '  export PATH="%s:\$PATH"\n' "${INSTALL_DIR}"
    printf '\n'
    ;;
esac

info "Run 'kaelo --help' to get started."
