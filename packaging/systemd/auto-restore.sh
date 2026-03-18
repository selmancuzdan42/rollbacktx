#!/bin/sh
# /usr/lib/rollbackx/auto-restore.sh
# auto-restore.conf varsa snapshot'ı geri yükler; reboot YAPILMAZ, boot devam eder.
# rollbackx-auto-restore.service tarafından erken boot aşamasında çalıştırılır.

set -e

AUTO="/var/lib/rollbackx/auto-restore.conf"
LOG="/var/log/rollbackx-restore.log"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] [oto] $*" | tee -a "$LOG"; }

[ -f "$AUTO" ] || { log "auto-restore.conf yok, çıkılıyor."; exit 0; }

SNAP_PATH=$(cat "$AUTO" | tr -d '[:space:]')

if [ -z "$SNAP_PATH" ] || [ ! -d "$SNAP_PATH" ]; then
    log "HATA: Snapshot dizini bulunamadı: '$SNAP_PATH'"
    exit 1
fi

log "=========================================="
log "Önyükleme otomatik geri yükleme: $SNAP_PATH"
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

log "Otomatik geri yükleme tamamlandı."

# GRUB menüsünü güncelle — yeni snapshot'lar görünsün
update-grub >> "$LOG" 2>&1 || true
log "GRUB güncellendi. Boot devam ediyor."
# Reboot YOK — sistemin kalanı geri yüklenen dosyalarla boot etsin
