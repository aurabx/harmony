#!/usr/bin/env bash
set -euo pipefail

# install-dev.sh - Build and install harmony binary to PATH for testing
# Usage: ./install-dev.sh [--profile <release|profiling>]

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="${HOME}/.local/bin"
BINARY_NAME="harmony"

# Default build profile
PROFILE="release"

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    *)
      echo "Unknown option: $1"
      echo "Usage: $0 [--profile <release|profiling>]"
      exit 1
      ;;
  esac
done

echo "🔨 Building harmony (profile: ${PROFILE})..."
cd "${SCRIPT_DIR}"

if [[ "${PROFILE}" == "release" ]]; then
  cargo build --release
  BINARY_PATH="target/release/${BINARY_NAME}"
else
  cargo build --profile "${PROFILE}"
  BINARY_PATH="target/${PROFILE}/${BINARY_NAME}"
fi

# Ensure install directory exists
mkdir -p "${INSTALL_DIR}"

# Install binary
echo "📦 Installing ${BINARY_NAME} to ${INSTALL_DIR}..."
cp "${BINARY_PATH}" "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

# On macOS, remove extended attributes that trigger Gatekeeper
if [[ "$(uname)" == "Darwin" ]]; then
  xattr -c "${INSTALL_DIR}/${BINARY_NAME}" 2>/dev/null || true
fi

# Verify installation
INSTALLED_PATH="$(which ${BINARY_NAME} 2>/dev/null || echo '')"
if [[ -z "${INSTALLED_PATH}" ]]; then
  echo "⚠️  Warning: ${INSTALL_DIR} may not be in your PATH"
  echo "   Add this to your shell profile: export PATH=\"${INSTALL_DIR}:\$PATH\""
elif [[ "${INSTALLED_PATH}" != "${INSTALL_DIR}/${BINARY_NAME}" ]]; then
  echo "⚠️  Warning: 'which ${BINARY_NAME}' points to ${INSTALLED_PATH}"
  echo "   Expected: ${INSTALL_DIR}/${BINARY_NAME}"
  echo "   Check your PATH order"
else
  echo "✅ Successfully installed to ${INSTALLED_PATH}"
  "${BINARY_NAME}" --version 2>/dev/null || echo "   (version command not available)"
fi
