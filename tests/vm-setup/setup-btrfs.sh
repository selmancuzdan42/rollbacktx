#!/usr/bin/env bash
# RollbackX Btrfs test ortamı kurulum scripti
# Root yetkisi gerektirir.
# Kullanım: sudo bash tests/vm-setup/setup-btrfs.sh

set -euo pipefail

IMG="/tmp/rx-test-btrfs.img"
LOOP="/dev/loop9"
MOUNT="/mnt/rx-btrfs-test"

echo "==> Btrfs test ortamı hazırlanıyor..."

# Önceki kalıntıları temizle
if mountpoint -q "$MOUNT" 2>/dev/null; then
    echo "  → $MOUNT bağlantısı kesiliyor..."
    umount -R "$MOUNT"
fi
if losetup "$LOOP" &>/dev/null; then
    echo "  → $LOOP serbest bırakılıyor..."
    losetup -d "$LOOP"
fi

# 2 GB loop device imajı
echo "  → 2 GB imaj oluşturuluyor: $IMG"
fallocate -l 2G "$IMG"
losetup "$LOOP" "$IMG"

# Btrfs formatla
echo "  → Btrfs formatlanıyor..."
mkfs.btrfs -f "$LOOP"

# Kök subvolume '@' oluştur (Snapper uyumlu)
mkdir -p "$MOUNT"
mount "$LOOP" "$MOUNT"
btrfs subvolume create "$MOUNT/@"
btrfs subvolume create "$MOUNT/@snapshots"

# set-default
ROOT_ID=$(btrfs subvolume list "$MOUNT" | awk '/ path @$/{print $2}')
btrfs subvolume set-default "$ROOT_ID" "$MOUNT"
umount "$MOUNT"

# @ subvolume'ü kök olarak mount et, @snapshots'ı /.snapshots olarak
mount -o subvol=@ "$LOOP" "$MOUNT"
mkdir -p "$MOUNT/.snapshots"
mount -o subvol=@snapshots "$LOOP" "$MOUNT/.snapshots"

echo ""
echo "==> Test ortamı hazır!"
echo "    Kök:       $MOUNT"
echo "    Snapshots: $MOUNT/.snapshots"
echo ""
echo "Testleri çalıştırmak için:"
echo "    ROLLBACKX_BTRFS_ROOT=$MOUNT ROLLBACKX_SNAP_DIR=$MOUNT/.snapshots \\"
echo "    cargo test --test btrfs_snapshot_test -- --nocapture"
echo ""
echo "Temizlik için:"
echo "    umount -R $MOUNT && losetup -d $LOOP && rm $IMG"
