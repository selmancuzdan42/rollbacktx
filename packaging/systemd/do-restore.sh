#!/bin/sh
# /usr/lib/rollbackx/do-restore.sh
# pending-restore.conf dosyasındaki snapshot yolunu okur ve geri yükler.
# rollbackx-restore.service tarafından erken boot aşamasında çalıştırılır.

set -e

PENDING="/var/lib/rollbackx/pending-restore.conf"
LOG="/var/log/rollbackx-restore.log"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

[ -f "$PENDING" ] || { log "pending-restore.conf yok, çıkılıyor."; exit 0; }

SNAP_PATH=$(cat "$PENDING" | tr -d '[:space:]')

if [ -z "$SNAP_PATH" ] || [ ! -d "$SNAP_PATH" ]; then
    log "HATA: Snapshot dizini bulunamadı: '$SNAP_PATH'"
    rm -f "$PENDING"
    exit 1
fi

log "=========================================="
log "Restore başlıyor: $SNAP_PATH"
log "=========================================="

rsync -aHAXx --delete \
    --exclude='/proc/' \
    --exclude='/sys/' \
    --exclude='/dev/' \
    --exclude='/run/' \
    --exclude='/tmp/' \
    --exclude='/var/lib/rollbackx/' \
    --exclude='/var/log/rollbackx-restore.log' \
    --exclude='/lost+found' \
    --exclude='/boot/grub/' \
    --exclude='/etc/systemd/system/sysinit.target.wants/rollbackx-auto-restore.service' \
    --exclude='/usr/bin/rollbackx' \
    --exclude='/usr/bin/rollbackx-gtk' \
    --exclude='/usr/lib/rollbackx/' \
    --exclude='/lib/systemd/system/rollbackx*' \
    --exclude='/usr/share/polkit-1/actions/org.rollbackx.policy' \
    --exclude='/usr/share/polkit-1/rules.d/org.rollbackx.rules' \
    --exclude='/etc/apt/apt.conf.d/80rollbackx' \
    --exclude='/etc/grub.d/80_rollbackx' \
    --exclude='/usr/share/applications/rollbackx.desktop' \
    --exclude='/etc/xdg/autostart/rollbackx-gtk.desktop' \
    --exclude='/usr/share/icons/hicolor/scalable/apps/rollbackx.svg' \
    "$SNAP_PATH/" / >> "$LOG" 2>&1

log "Restore tamamlandı."

# Exclude edilen RollbackX dosyaları eksikse snapshot'tan geri kopyala
for f in /etc/grub.d/80_rollbackx /etc/apt/apt.conf.d/80rollbackx \
         /usr/share/applications/rollbackx.desktop /etc/xdg/autostart/rollbackx-gtk.desktop \
         /usr/share/polkit-1/actions/org.rollbackx.policy \
         /usr/share/icons/hicolor/scalable/apps/rollbackx.svg; do
    if [ ! -f "$f" ] && [ -f "${SNAP_PATH}${f}" ]; then
        mkdir -p "$(dirname "$f")"
        cp "${SNAP_PATH}${f}" "$f"
        log "Eksik dosya geri yüklendi: $f"
    fi
done
[ -f /etc/grub.d/80_rollbackx ] && chmod 755 /etc/grub.d/80_rollbackx

# GRUB menüsünü güncelle — yeni snapshot'lar görünsün
update-grub >> "$LOG" 2>&1 || true
log "GRUB güncellendi."

# Marker'ı sil — bir daha çalışmasın
rm -f "$PENDING"

log "Sistem yeniden başlatılıyor..."
systemctl reboot
