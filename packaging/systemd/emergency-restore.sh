#!/bin/sh
# /usr/lib/rollbackx/emergency-restore.sh
# GRUB'dan init= parametresi ile PID 1 olarak calisir.
# systemd'yi BYPASS eder — /etc silinmis bile calisir.

# Root FS'yi yazılabilir yap
mount -o remount,rw /
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sysfs /sys 2>/dev/null
mount -t devtmpfs devtmpfs /dev 2>/dev/null

# Kernel console mesajlarini kapat (progress bar'i bozmasin)
dmesg -n 1 2>/dev/null

LOG="/var/log/rollbackx-restore.log"
DB="/var/lib/rollbackx/snapshots.json"

# ANSI renk kodlari
GREEN="\033[1;32m"
RED="\033[1;31m"
YELLOW="\033[1;33m"
CYAN="\033[1;36m"
WHITE="\033[1;37m"
DIM="\033[2m"
RESET="\033[0m"
CLEAR="\033[2J\033[H"

log() { echo "$*" >> "$LOG" 2>/dev/null; }

# Ekrani temizle
printf "$CLEAR"
printf "\n"
printf "${DIM}══════════════════════════════════════════════${RESET}\n"
printf "\n"
printf "   ${GREEN}██████${RESET}  ${WHITE}RollbackX Acil Geri Yukleme${RESET}\n"
printf "   ${GREEN}██  ██${RESET}\n"
printf "   ${GREEN}██████${RESET}  ${DIM}Sistem kurtarma modu${RESET}\n"
printf "\n"
printf "${DIM}══════════════════════════════════════════════${RESET}\n"
printf "\n"

log "=========================================="
log "RollbackX Acil Geri Yukleme baslatildi"
log "=========================================="

# Kernel parametresinden restore ID'yi oku
RESTORE_ID=""
for param in $(cat /proc/cmdline); do
    case "$param" in
        rollbackx.restore_id=*)
            RESTORE_ID="${param#rollbackx.restore_id=}"
            ;;
    esac
done

if [ -z "$RESTORE_ID" ]; then
    printf "   ${RED}✗${RESET} rollbackx.restore_id parametresi bulunamadi.\n"
    printf "   ${DIM}Normal boot'a geciliyor...${RESET}\n"
    sleep 3
    umount /proc /sys /dev 2>/dev/null
    exec /sbin/init
fi

printf "   ${CYAN}→${RESET} Snapshot ID: ${WHITE}#${RESTORE_ID}${RESET}\n"
log "Restore ID: $RESTORE_ID"

# DB'den snapshot path ve adini bul (python3/jq'suz)
# Pretty-printed JSON'dan parse: "id": N satırını bul,
# sonraki satirlardan backend_ref ve name'i cek.
SNAP_PATH=""
SNAP_NAME=""
if [ -f "$DB" ]; then
    FOUND=0
    while IFS= read -r line; do
        if [ "$FOUND" = "0" ]; then
            # "id": 2 veya "id": 2, formatini ara
            case "$line" in
                *"\"id\""*":"*)
                    line_id=$(echo "$line" | sed 's/[^0-9]//g')
                    if [ "$line_id" = "$RESTORE_ID" ]; then
                        FOUND=1
                    fi
                    ;;
            esac
        else
            # id bulunduktan sonra backend_ref ve name'i ara
            case "$line" in
                *"\"backend_ref\""*)
                    SNAP_REF=$(echo "$line" | sed 's/.*"backend_ref"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
                    if [ -n "$SNAP_REF" ]; then
                        SNAP_PATH="/var/lib/rollbackx/snapshots/$SNAP_REF"
                    fi
                    ;;
                *"\"name\""*)
                    SNAP_NAME=$(echo "$line" | sed 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
                    ;;
                # Sonraki objeye gectik, dur
                *"\"id\""*)
                    break
                    ;;
            esac
            # Ikisinide bulduysa dur
            if [ -n "$SNAP_PATH" ] && [ -n "$SNAP_NAME" ]; then
                break
            fi
        fi
    done < "$DB"
fi

if [ -z "$SNAP_PATH" ]; then
    printf "   ${RED}✗${RESET} Snapshot bulunamadi.\n"
    printf "   ${DIM}Normal boot'a geciliyor...${RESET}\n"
    log "HATA: ID=$RESTORE_ID icin snapshot bulunamadi."
    sleep 3
    umount /proc /sys /dev 2>/dev/null
    exec /sbin/init
fi

if [ ! -d "$SNAP_PATH" ]; then
    printf "   ${RED}✗${RESET} Snapshot dizini yok: ${SNAP_PATH}\n"
    printf "   ${DIM}Normal boot'a geciliyor...${RESET}\n"
    log "HATA: Snapshot dizini yok: $SNAP_PATH"
    sleep 3
    umount /proc /sys /dev 2>/dev/null
    exec /sbin/init
fi

# Kernel mesajlarini sustir ve ekrani tekrar temizle
dmesg -n 1 2>/dev/null
printf "$CLEAR"
printf "\n"
printf "${DIM}══════════════════════════════════════════════${RESET}\n"
printf "\n"
printf "   ${GREEN}██████${RESET}  ${WHITE}RollbackX Acil Geri Yukleme${RESET}\n"
printf "   ${GREEN}██  ██${RESET}\n"
printf "   ${GREEN}██████${RESET}  ${DIM}Sistem kurtarma modu${RESET}\n"
printf "\n"
printf "${DIM}══════════════════════════════════════════════${RESET}\n"
printf "\n"
printf "   ${CYAN}→${RESET} Snapshot ID: ${WHITE}#${RESTORE_ID}${RESET}\n"
if [ -n "$SNAP_NAME" ]; then
    printf "   ${CYAN}→${RESET} Snapshot: ${WHITE}${SNAP_NAME}${RESET}\n"
fi
printf "   ${CYAN}→${RESET} Kaynak:   ${DIM}${SNAP_PATH}${RESET}\n"
printf "\n"
printf "   ${YELLOW}Geri yukleme basliyor...${RESET}\n"
printf "\n"

log "Snapshot: $SNAP_NAME ($SNAP_PATH)"
log "rsync basliyor..."

# Progress bar fonksiyonu
# rsync --info=progress2 ciktisi: "  1,234,567  45%  12.34MB/s ..."
# Bunu parse edip gorsel bar ciziyoruz.

BAR_WIDTH=40

draw_bar() {
    pct=$1
    filled=$((pct * BAR_WIDTH / 100))
    empty=$((BAR_WIDTH - filled))

    bar=""
    i=0
    while [ $i -lt $filled ]; do
        bar="${bar}█"
        i=$((i + 1))
    done
    while [ $i -lt $BAR_WIDTH ]; do
        bar="${bar}░"
        i=$((i + 1))
    done

    if [ "$pct" -lt 30 ]; then
        COLOR="$RED"
    elif [ "$pct" -lt 70 ]; then
        COLOR="$YELLOW"
    else
        COLOR="$GREEN"
    fi

    printf "\r   ${COLOR}[${bar}]${RESET}  ${WHITE}%3d%%${RESET}  " "$pct"
}

# rsync calistir, progress'i parse et
rsync -aHAXx --delete --info=progress2 --no-inc-recursive \
    --exclude='/proc/' \
    --exclude='/sys/' \
    --exclude='/dev/' \
    --exclude='/run/' \
    --exclude='/tmp/' \
    --exclude='/var/lib/rollbackx/' \
    --exclude='/var/log/rollbackx-restore.log' \
    --exclude='/lost+found' \
    --exclude='/boot/grub/' \
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
    "$SNAP_PATH/" / 2>>"$LOG" | while IFS= read -r line; do
        # rsync --info=progress2 satirindan yuzdeyi cek
        pct=$(echo "$line" | sed -n 's/.*[[:space:]]\([0-9]*\)%.*/\1/p')
        if [ -n "$pct" ] && [ "$pct" -ge 0 ] 2>/dev/null && [ "$pct" -le 100 ] 2>/dev/null; then
            draw_bar "$pct"
        fi
    done

RESULT=$?

printf "\n\n"

if [ $RESULT -eq 0 ] || [ $RESULT -eq 24 ]; then
    printf "   ${GREEN}✓ Geri yukleme basarili!${RESET}\n"
    log "Geri yukleme BASARILI."
else
    printf "   ${YELLOW}⚠ rsync hata kodu: ${RESULT} — sistem yine de baslatiliyor.${RESET}\n"
    log "UYARI: rsync hata kodu: $RESULT"
fi

printf "\n"
printf "   ${DIM}Sistem 3 saniye icinde yeniden basliyor...${RESET}\n"
printf "\n"
printf "${DIM}══════════════════════════════════════════════${RESET}\n"

log "Sistem baslatiliyor..."

sleep 3

# Temizlik
umount /proc /sys /dev 2>/dev/null

# Gercek init'e (systemd) gec
exec /sbin/init
