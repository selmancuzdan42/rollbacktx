#!/bin/sh
# /usr/lib/rollbackx/cmdline-restore.sh
# GRUB'dan gelen rollbackx.restore_id=<id> kernel parametresini okur
# ve pending-restore.conf dosyasını oluşturur.
# rollbackx-restore.service tarafından tüketilir.

set -e

DB="/var/lib/rollbackx/snapshots.json"
PENDING="/var/lib/rollbackx/pending-restore.conf"
LOG="/var/log/rollbackx-restore.log"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "$LOG"; }

log "GRUB cmdline restore servisi başlatıldı."

# Emergency restore sonrası ertelenmiş GRUB güncellemesi
GRUB_PENDING="/var/lib/rollbackx/grub-update-pending"
if [ -f "$GRUB_PENDING" ]; then
    log "Ertelenmiş GRUB güncellemesi yapılıyor..."
    update-grub >> "$LOG" 2>&1 || true
    rm -f "$GRUB_PENDING"
    log "GRUB güncellendi."
fi

# rollbackx.restore_id parametresini /proc/cmdline'dan çıkar
RESTORE_ID=$(cat /proc/cmdline | tr ' ' '\n' | grep '^rollbackx\.restore_id=' | cut -d= -f2 | head -1)

if [ -z "$RESTORE_ID" ]; then
    log "rollbackx.restore_id parametresi bulunamadı, çıkılıyor."
    exit 0
fi

log "Restore ID: $RESTORE_ID"

# DB'yi oku ve snapshot path'i bul
if [ ! -f "$DB" ]; then
    log "HATA: Snapshot DB bulunamadı: $DB"
    exit 1
fi

if command -v python3 >/dev/null 2>&1; then
    SNAP_PATH=$(python3 -c "
import json, sys
data = json.load(open('$DB'))
for s in data:
    if str(s['id']) == '$RESTORE_ID':
        ref = s['backend_ref']
        backend = s['backend']
        if backend == 'Rsync':
            print('/var/lib/rollbackx/snapshots/' + ref)
        else:
            print(ref)
        sys.exit(0)
sys.exit(1)
" 2>/dev/null)
else
    log "HATA: python3 bulunamadı."
    exit 1
fi

if [ -z "$SNAP_PATH" ]; then
    log "HATA: ID=$RESTORE_ID için snapshot bulunamadı."
    exit 1
fi

if [ ! -d "$SNAP_PATH" ]; then
    log "HATA: Snapshot dizini yok: $SNAP_PATH"
    exit 1
fi

log "Snapshot dizini: $SNAP_PATH"
log "pending-restore.conf yazılıyor..."
echo "$SNAP_PATH" > "$PENDING"

log "Hazır. rollbackx-restore.service restore işlemini yürütecek."
exit 0
