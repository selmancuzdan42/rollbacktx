#!/bin/bash
# RollbackX — tam release build scripti
# Pardus/Debian üzerinde çalıştırın:
#   bash scripts/build-release.sh
#
# Çıktı: rollbackx_0.1.0_amd64.deb  (hem CLI hem GTK içerir)
set -e

KIRMIZI='\033[0;31m'
YESIL='\033[0;32m'
MAVI='\033[0;34m'
SIFIR='\033[0m'

baslik() { echo -e "\n${MAVI}==> $1${SIFIR}"; }
tamam()  { echo -e "    ${YESIL}✓ $1${SIFIR}"; }
hata()   { echo -e "    ${KIRMIZI}✗ $1${SIFIR}"; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$ROOT_DIR"

# ── 1. Build bağımlılıkları ───────────────────────────────────────────────────
baslik "Build bağımlılıkları kontrol ediliyor..."

sudo apt-get update -qq

PKGS=""
command -v cc        >/dev/null 2>&1 || PKGS="$PKGS build-essential"
command -v musl-gcc  >/dev/null 2>&1 || PKGS="$PKGS musl-tools"
pkg-config --exists gtk4          2>/dev/null || PKGS="$PKGS libgtk-4-dev"
pkg-config --exists libadwaita-1  2>/dev/null || PKGS="$PKGS libadwaita-1-dev"
pkg-config --exists dbus-1        2>/dev/null || PKGS="$PKGS libdbus-1-dev"
command -v dpkg-deb  >/dev/null 2>&1 || PKGS="$PKGS dpkg-dev"
command -v pkg-config >/dev/null 2>&1 || PKGS="$PKGS pkg-config"
command -v rsync     >/dev/null 2>&1 || PKGS="$PKGS rsync"

if [ -n "$PKGS" ]; then
    echo "    Kuruluyor:$PKGS"
    sudo apt-get install -y --no-install-recommends $PKGS
    tamam "Bağımlılıklar kuruldu"
else
    tamam "Tüm bağımlılıklar mevcut"
fi

# ── 2. Rust araç zinciri ──────────────────────────────────────────────────────
baslik "Rust araç zinciri kontrol ediliyor..."

if ! command -v cargo >/dev/null 2>&1; then
    echo "    Rust kurulu değil, kuruluyor..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    tamam "Rust kuruldu"
else
    tamam "Rust: $(rustc --version)"
fi

if ! rustup target list --installed | grep -q x86_64-unknown-linux-musl; then
    echo "    musl target ekleniyor..."
    rustup target add x86_64-unknown-linux-musl
    tamam "musl target eklendi"
else
    tamam "musl target mevcut"
fi

# ── 3. CLI derleme (musl static) ──────────────────────────────────────────────
baslik "CLI derleniyor (musl static — glibc bağımsız)..."
cargo build --release --target x86_64-unknown-linux-musl -p rollbackx-cli
tamam "CLI derlendi: $(du -sh target/x86_64-unknown-linux-musl/release/rollbackx | cut -f1)"

# ── 4. GTK derleme ────────────────────────────────────────────────────────────
baslik "GTK arayüzü derleniyor..."
cargo build --release -p rollbackx-gtk
tamam "GTK derlendi: $(du -sh target/release/rollbackx-gtk | cut -f1)"

# ── 5. .deb paketi ────────────────────────────────────────────────────────────
baslik ".deb paketi oluşturuluyor..."
make deb
tamam ".deb hazır: $(du -sh rollbackx_0.1.0_amd64.deb | cut -f1)"

# ── 6. Özet ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${YESIL}╔══════════════════════════════════════════════╗${SIFIR}"
echo -e "${YESIL}║   RollbackX build tamamlandı!               ║${SIFIR}"
echo -e "${YESIL}╠══════════════════════════════════════════════╣${SIFIR}"
echo -e "${YESIL}║${SIFIR}  Paket : rollbackx_0.1.0_amd64.deb          ${YESIL}║${SIFIR}"
echo -e "${YESIL}║${SIFIR}  Kurmak: sudo dpkg -i rollbackx_0.1.0_amd64.deb ${YESIL}║${SIFIR}"
echo -e "${YESIL}╚══════════════════════════════════════════════╝${SIFIR}"
