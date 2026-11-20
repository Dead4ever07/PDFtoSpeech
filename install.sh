#!/usr/bin/env bash
#
# install.sh - Installer for PDFtoSpeech (Tauri + Vue + TypeScript)
# Checks & installs dependencies, compiles the project, and installs it.
#
set -euo pipefail

# ANSI color codes
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m' # No Color

log_info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
log_step()  { echo -e "\n${CYAN}${BOLD}==> $*${NC}"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
log_error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# Ensure we run from the project root directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Defaults
INSTALL_MODE="auto" # "auto", "system", "user"
SKIP_DEPS=false
NO_BUNDLE=false

usage() {
  cat << EOF
Usage: $0 [OPTIONS]

PDFtoSpeech Installer: Checks/installs dependencies, builds, and installs the application.

Options:
  --system       Install system-wide (requires sudo)
  --user         Install for the current user only (~/.local)
  --skip-deps    Skip checking/installing system dependencies
  --no-bundle    Compile release binary without creating deb/appimage packages
  -h, --help     Show this help message
EOF
  exit 0
}

# Parse command line options
while [ $# -gt 0 ]; do
  case "$1" in
    --system)
      INSTALL_MODE="system"
      shift
      ;;
    --user)
      INSTALL_MODE="user"
      shift
      ;;
    --skip-deps)
      SKIP_DEPS=true
      shift
      ;;
    --no-bundle)
      NO_BUNDLE=true
      shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      log_warn "Unknown option: $1"
      shift
      ;;
  esac
done

# Warn if executed directly as root with sudo
if [ "$EUID" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
  log_warn "Running as root via sudo. It is usually best to run './install.sh' as a standard user;"
  log_warn "the script will request sudo only when installing system packages."
fi

# Privilege helpers
has_sudo() {
  if [ "$EUID" -eq 0 ]; then
    return 0
  fi
  if command -v sudo >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

run_root() {
  if [ "$EUID" -eq 0 ]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    log_error "Root privileges (sudo) required to run: $*"
    return 1
  fi
}

# Architecture detection
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)
    ARCH_PIPER="x86_64"
    ARCH_PDFIUM="x64"
    ARCH_RUST="x86_64"
    ;;
  aarch64|arm64)
    ARCH_PIPER="aarch64"
    ARCH_PDFIUM="arm64"
    ARCH_RUST="aarch64"
    ;;
  *)
    log_warn "Architecture '$ARCH' may not have official prebuilt piper/pdfium binaries."
    ARCH_PIPER="$ARCH"
    ARCH_PDFIUM="$ARCH"
    ARCH_RUST="$ARCH"
    ;;
esac

# ==============================================================================
# Step 1: System Dependencies
# ==============================================================================
if [ "$SKIP_DEPS" = false ]; then
  log_step "Checking system packages..."
  if [ -f /etc/os-release ]; then
    . /etc/os-release
    OS_ID="${ID:-unknown}"
    OS_ID_LIKE="${ID_LIKE:-}"
  else
    OS_ID="unknown"
    OS_ID_LIKE=""
  fi

  if [[ "$OS_ID" =~ ^(ubuntu|debian|linuxmint|pop)$ ]] || [[ "$OS_ID_LIKE" =~ (ubuntu|debian) ]]; then
    log_info "Detected Debian/Ubuntu-based distribution ($OS_ID)."
    REQUIRED_PKGS=(
      build-essential
      curl
      wget
      file
      pkg-config
      libssl-dev
      libgtk-3-dev
      libwebkit2gtk-4.1-dev
      libayatana-appindicator3-dev
      librsvg2-dev
      libxdo-dev
    )

    MISSING_PKGS=()
    for pkg in "${REQUIRED_PKGS[@]}"; do
      if ! dpkg -s "$pkg" 2>/dev/null | grep -q "Status: install ok installed"; then
        MISSING_PKGS+=("$pkg")
      fi
    done

    if [ ${#MISSING_PKGS[@]} -gt 0 ]; then
      log_info "Installing missing system packages: ${MISSING_PKGS[*]}"
      run_root apt-get update
      run_root apt-get install -y "${MISSING_PKGS[@]}"
    else
      log_info "All required system packages are installed."
    fi
  elif [[ "$OS_ID" =~ ^(fedora|rhel|centos)$ ]] || [[ "$OS_ID_LIKE" =~ (fedora|rhel) ]]; then
    log_info "Detected Fedora/RHEL-based distribution ($OS_ID)."
    FEDORA_PKGS=(
      gcc gcc-c++ make curl wget file pkgconf-pkg-config
      openssl-devel gtk3-devel webkit2gtk4.1-devel
      libappindicator-gtk3-devel librsvg2-devel
    )
    run_root dnf install -y "${FEDORA_PKGS[@]}"
  elif [[ "$OS_ID" =~ ^(arch|manjaro)$ ]] || [[ "$OS_ID_LIKE" =~ arch ]]; then
    log_info "Detected Arch-based distribution ($OS_ID)."
    ARCH_PKGS=(
      base-devel curl wget file pkgconf
      openssl gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg
    )
    run_root pacman -S --needed --noconfirm "${ARCH_PKGS[@]}"
  else
    log_warn "Unrecognized Linux distribution ($OS_ID). Please ensure GTK3, WebKit2GTK 4.1, and build tools are installed."
  fi
else
  log_info "Skipping system dependencies check (--skip-deps specified)."
fi

# ==============================================================================
# Step 2: Node.js & pnpm
# ==============================================================================
log_step "Checking Node.js & pnpm..."

# Load NVM if present in current or user home
if ! command -v node >/dev/null 2>&1; then
  if [ -s "$HOME/.nvm/nvm.sh" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.nvm/nvm.sh"
  elif [ -n "${SUDO_USER:-}" ] && [ -s "/home/$SUDO_USER/.nvm/nvm.sh" ]; then
    # shellcheck source=/dev/null
    . "/home/$SUDO_USER/.nvm/nvm.sh"
  fi
fi

if ! command -v node >/dev/null 2>&1; then
  log_warn "Node.js is not found. Attempting to install Node.js..."
  if command -v apt-get >/dev/null 2>&1; then
    run_root curl -fsSL https://deb.nodesource.com/setup_20.x | run_root bash -
    run_root apt-get install -y nodejs
  else
    log_error "Node.js (>= 18) is required. Please install Node.js and rerun this script."
    exit 1
  fi
fi
log_info "Node.js found: $(node -v)"

# Check pnpm
if ! command -v pnpm >/dev/null 2>&1; then
  if command -v corepack >/dev/null 2>&1; then
    log_info "Enabling pnpm via corepack..."
    export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
    corepack enable >/dev/null 2>&1 || true
  fi
fi

if ! command -v pnpm >/dev/null 2>&1; then
  if command -v npm >/dev/null 2>&1; then
    log_info "Installing pnpm globally via npm..."
    if [ "$EUID" -eq 0 ]; then
      npm install -g pnpm
    elif has_sudo; then
      run_root npm install -g pnpm
    else
      npm install -g pnpm || curl -fsSL https://get.pnpm.io/install.sh | sh -
    fi
  else
    log_info "Installing pnpm via standalone installer..."
    curl -fsSL https://get.pnpm.io/install.sh | sh -
    export PNPM_HOME="$HOME/.local/share/pnpm"
    export PATH="$PNPM_HOME:$PATH"
  fi
fi

if ! command -v pnpm >/dev/null 2>&1; then
  log_error "pnpm is required. Please install pnpm and rerun this script."
  exit 1
fi
log_info "pnpm found: $(pnpm -v)"

# ==============================================================================
# Step 3: Rust & Cargo
# ==============================================================================
log_step "Checking Rust toolchain..."

# Source Cargo env if present
if [ -s "$HOME/.cargo/env" ]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
elif [ -n "${SUDO_USER:-}" ] && [ -s "/home/$SUDO_USER/.cargo/env" ]; then
  # shellcheck source=/dev/null
  . "/home/$SUDO_USER/.cargo/env"
fi

if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
  log_info "Rust toolchain not found. Installing via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  if [ -s "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
  fi
fi

if ! command -v cargo >/dev/null 2>&1; then
  log_error "Cargo could not be found. Please ensure ~/.cargo/bin is in your PATH."
  exit 1
fi
log_info "Rust compiler found: $(rustc --version)"
log_info "Cargo found: $(cargo --version)"

# ==============================================================================
# Step 4: Runtime Assets (libpdfium, Piper TTS, Voice models)
# ==============================================================================
log_step "Checking runtime assets in src-tauri/resources..."

RESOURCES_DIR="src-tauri/resources"
mkdir -p "$RESOURCES_DIR/piper"
mkdir -p "$RESOURCES_DIR/models/alba"
mkdir -p "$RESOURCES_DIR/models/joe"

# 1. libpdfium.so
PDFIUM_SO="$RESOURCES_DIR/libpdfium.so"
if [ ! -s "$PDFIUM_SO" ] || [ "$(wc -c < "$PDFIUM_SO")" -lt 1000000 ]; then
  log_info "libpdfium.so missing or invalid. Restoring/downloading..."
  # Try restoring from local git commit history if available
  if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    git show bffe688:src-tauri/libpdfium.so > "$PDFIUM_SO" 2>/dev/null || true
  fi

  # Fallback to downloading official pdfium prebuilt binaries
  if [ ! -s "$PDFIUM_SO" ] || [ "$(wc -c < "$PDFIUM_SO")" -lt 1000000 ]; then
    log_info "Downloading libpdfium for $ARCH_PDFIUM..."
    TMP_DIR=$(mktemp -d)
    PDFIUM_URL="https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-linux-${ARCH_PDFIUM}.tgz"
    if curl -sSL "$PDFIUM_URL" | tar -xz -C "$TMP_DIR"; then
      if [ -f "$TMP_DIR/lib/libpdfium.so" ]; then
        cp "$TMP_DIR/lib/libpdfium.so" "$PDFIUM_SO"
      fi
    fi
    rm -rf "$TMP_DIR"
  fi
  chmod +x "$PDFIUM_SO"
fi
log_info "libpdfium.so: OK"

# 2. Piper TTS binary and runtime libraries
PIPER_BIN="$RESOURCES_DIR/piper/piper"
if [ ! -x "$PIPER_BIN" ]; then
  log_info "Downloading Piper TTS standalone bundle for $ARCH_PIPER..."
  PIPER_URL="https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_linux_${ARCH_PIPER}.tar.gz"
  curl -sSL "$PIPER_URL" | tar -xz -C "$RESOURCES_DIR/"
  chmod +x "$PIPER_BIN"
fi
log_info "Piper TTS binary: OK"

# 3. Voice model: Alba
ALBA_ONNX="$RESOURCES_DIR/models/alba/voice.onnx"
if [ ! -s "$ALBA_ONNX" ]; then
  log_info "Downloading Alba voice model (.onnx)..."
  ALBA_URL="https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/alba/medium/en_GB-alba-medium.onnx"
  curl -sSL "$ALBA_URL" -o "$ALBA_ONNX"
fi
log_info "Voice model Alba: OK"

# 4. Voice model: Joe
JOE_ONNX="$RESOURCES_DIR/models/joe/voice.onnx"
if [ ! -s "$JOE_ONNX" ]; then
  log_info "Downloading Joe voice model (.onnx)..."
  JOE_URL="https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/joe/medium/en_US-joe-medium.onnx"
  curl -sSL "$JOE_URL" -o "$JOE_ONNX"
fi
log_info "Voice model Joe: OK"

# ==============================================================================
# Step 5: Install Project Dependencies
# ==============================================================================
log_step "Installing project dependencies via pnpm..."
if pnpm install --help 2>&1 | grep -q -- "--dangerously-allow-all-builds"; then
  pnpm install --dangerously-allow-all-builds
else
  pnpm install
fi

# ==============================================================================
# Step 6: Compile Project
# ==============================================================================
log_step "Compiling project..."

# Build frontend first
log_info "Building frontend..."
pnpm build

# Build Tauri application
log_info "Compiling Tauri application..."
if [ "$NO_BUNDLE" = true ]; then
  log_info "Building release binary (--no-bundle)..."
  pnpm tauri build --no-bundle
else
  log_info "Building release package..."
  if ! pnpm tauri build; then
    log_warn "Full bundling encountered an issue. Attempting Debian bundle only (--bundles deb)..."
    if ! pnpm tauri build --bundles deb; then
      log_warn "Debian bundle failed. Building standalone release binary (--no-bundle)..."
      pnpm tauri build --no-bundle
    fi
  fi
fi

# ==============================================================================
# Step 7: Install Project
# ==============================================================================
log_step "Installing application..."

DEB_PKG=$(find src-tauri/target/release/bundle/deb -name "*.deb" 2>/dev/null | head -n 1)

if [ "$INSTALL_MODE" != "user" ] && [ -n "$DEB_PKG" ] && [ -f "$DEB_PKG" ] && has_sudo; then
  log_info "Installing via Debian package: $DEB_PKG"
  if command -v apt-get >/dev/null 2>&1; then
    run_root apt-get install -y "$DEB_PKG" || run_root dpkg -i "$DEB_PKG"
  else
    run_root dpkg -i "$DEB_PKG"
  fi
  log_info "Debian package successfully installed."
else
  BINARY_SRC="src-tauri/target/release/PDFtoSpeech"
  if [ ! -f "$BINARY_SRC" ]; then
    log_error "Compiled binary not found at $BINARY_SRC"
    exit 1
  fi

  if [ "$INSTALL_MODE" = "user" ] || ! has_sudo; then
    PREFIX="$HOME/.local"
    USE_ROOT=false
    log_info "Installing for current user into $PREFIX..."
  else
    PREFIX="/usr/local"
    USE_ROOT=true
    log_info "Installing system-wide into $PREFIX..."
  fi

  BIN_DIR="$PREFIX/bin"
  APPS_DIR="$PREFIX/share/applications"
  ICONS_DIR="$PREFIX/share/icons/hicolor/128x128/apps"
  DATA_DIR="$PREFIX/share/PDFtoSpeech"

  if [ "$USE_ROOT" = true ]; then
    run_root mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICONS_DIR" "$DATA_DIR"
    run_root cp "$BINARY_SRC" "$BIN_DIR/PDFtoSpeech"
    run_root chmod 755 "$BIN_DIR/PDFtoSpeech"
    run_root cp -r "$RESOURCES_DIR" "$DATA_DIR/"
    run_root cp src-tauri/icons/128x128.png "$ICONS_DIR/pdftospeech.png"

    cat << 'EOF' | run_root tee "$APPS_DIR/PDFtoSpeech.desktop" > /dev/null
[Desktop Entry]
Categories=Utility;Office;Audio;
Comment=PDF text-to-speech reader with highlighting
Exec=PDFtoSpeech %U
Icon=pdftospeech
Name=PDFtoSpeech
Terminal=false
Type=Application
EOF
    run_root chmod 644 "$APPS_DIR/PDFtoSpeech.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
      run_root update-desktop-database "$APPS_DIR" 2>/dev/null || true
    fi
  else
    mkdir -p "$BIN_DIR" "$APPS_DIR" "$ICONS_DIR" "$DATA_DIR"
    cp "$BINARY_SRC" "$BIN_DIR/PDFtoSpeech"
    chmod 755 "$BIN_DIR/PDFtoSpeech"
    cp -r "$RESOURCES_DIR" "$DATA_DIR/"
    cp src-tauri/icons/128x128.png "$ICONS_DIR/pdftospeech.png"

    cat << EOF > "$APPS_DIR/PDFtoSpeech.desktop"
[Desktop Entry]
Categories=Utility;Office;Audio;
Comment=PDF text-to-speech reader with highlighting
Exec=$BIN_DIR/PDFtoSpeech %U
Icon=$ICONS_DIR/pdftospeech.png
Name=PDFtoSpeech
Terminal=false
Type=Application
EOF
    chmod 644 "$APPS_DIR/PDFtoSpeech.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
      update-desktop-database "$APPS_DIR" 2>/dev/null || true
    fi
  fi
  log_info "Installed binary to $BIN_DIR/PDFtoSpeech"
fi

# Fix workspace file ownership if run via sudo
if [ "$EUID" -eq 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != "root" ]; then
  chown -R "$SUDO_USER:$(id -gn "$SUDO_USER")" "$SCRIPT_DIR" 2>/dev/null || true
fi

# ==============================================================================
# Step 8: Done
# ==============================================================================
log_step "Installation complete!"
echo -e "${GREEN}${BOLD}PDFtoSpeech has been successfully compiled and installed!${NC}"
echo ""
echo "You can launch PDFtoSpeech by:"
echo "  1. Selecting PDFtoSpeech from your desktop application launcher"
echo "  2. Running 'PDFtoSpeech' in your terminal"
echo ""
