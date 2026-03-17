#!/bin/bash
# RollbackX manuel kurulum scripti
# Kullanım: sudo bash install.sh
set -e

ROLLBACKX_BIN="${1:-/tmp/rollbackx}"
ROLLBACKX_GTK_BIN="${2:-/tmp/rollbackx-gtk}"

echo "==> RollbackX kuruluyor..."

# Binary'ler
install -Dm755 "$ROLLBACKX_BIN"     /usr/bin/rollbackx
[ -f "$ROLLBACKX_GTK_BIN" ] && install -Dm755 "$ROLLBACKX_GTK_BIN" /usr/bin/rollbackx-gtk

# Veri dizini
mkdir -p /var/lib/rollbackx
chmod 755 /var/lib/rollbackx
# Snapshot DB dünya tarafından okunabilir (GUI için)
touch /var/lib/rollbackx/snapshots.json
chmod 644 /var/lib/rollbackx/snapshots.json

# GRUB script
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
install -Dm755 "$SCRIPT_DIR/grub.d/80_rollbackx"      /etc/grub.d/80_rollbackx

# Otomatik başlatma (GUI)
install -Dm644 "$SCRIPT_DIR/rollbackx-gtk-autostart.desktop" \
    /etc/xdg/autostart/rollbackx-gtk.desktop

# Polkit
install -Dm644 "$SCRIPT_DIR/polkit/org.rollbackx.policy" \
    /usr/share/polkit-1/actions/org.rollbackx.policy

# Uygulama menüsü
install -Dm644 "$SCRIPT_DIR/rollbackx.desktop" \
    /usr/share/applications/rollbackx.desktop

# Systemd servisleri
mkdir -p /usr/lib/rollbackx
install -Dm755 "$SCRIPT_DIR/systemd/cmdline-restore.sh" \
    /usr/lib/rollbackx/cmdline-restore.sh
install -Dm755 "$SCRIPT_DIR/systemd/do-restore.sh" \
    /usr/lib/rollbackx/do-restore.sh
install -Dm644 "$SCRIPT_DIR/systemd/rollbackx-cmdline.service" \
    /lib/systemd/system/rollbackx-cmdline.service
install -Dm644 "$SCRIPT_DIR/systemd/rollbackx-restore.service" \
    /lib/systemd/system/rollbackx-restore.service

systemctl daemon-reload
systemctl enable rollbackx-cmdline.service
systemctl enable rollbackx-restore.service

# GRUB güncelle
echo "==> GRUB güncelleniyor..."
update-grub

echo ""
echo "==> Kurulum tamamlandı!"
echo "    CLI:  rollbackx --help"
echo "    GUI:  rollbackx-gtk"
echo "    GRUB: Snapshot'lar boot menüsünde görünecek."
