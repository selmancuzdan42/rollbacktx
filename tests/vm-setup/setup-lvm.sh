#!/usr/bin/env bash
# RollbackX LVM Thin test ortamı kurulum scripti
# Root yetkisi gerektirir.
# Kullanım: sudo bash tests/vm-setup/setup-lvm.sh

set -euo pipefail

IMG="/tmp/rx-test-lvm.img"
LOOP="/dev/loop8"
VG="rxvg"
POOL="${VG}/pool"
ROOT_LV="root"

echo "==> LVM Thin test ortamı hazırlanıyor..."

# Önceki kalıntıları temizle
if vgs "$VG" &>/dev/null; then
    echo "  → Mevcut VG '$VG' kaldırılıyor..."
    vgremove -f "$VG"
fi
if losetup "$LOOP" &>/dev/null; then
    echo "  → $LOOP serbest bırakılıyor..."
    losetup -d "$LOOP"
fi
[ -f "$IMG" ] && rm -f "$IMG"

# 4 GB loop device imajı
echo "  → 4 GB imaj oluşturuluyor: $IMG"
fallocate -l 4G "$IMG"
losetup "$LOOP" "$IMG"

# LVM yapısını kur
echo "  → PV oluşturuluyor..."
pvcreate "$LOOP"

echo "  → VG '$VG' oluşturuluyor..."
vgcreate "$VG" "$LOOP"

echo "  → Thin pool (2 GB) oluşturuluyor..."
lvcreate --thin -L 2G -n pool "$VG"

echo "  → Root LV (1 GB thin volume) oluşturuluyor..."
lvcreate --thin -V 1G -T "${VG}/pool" -n "$ROOT_LV"

echo "  → Root LV formatlanıyor (ext4)..."
mkfs.ext4 "/dev/${VG}/${ROOT_LV}"

echo ""
echo "==> Test ortamı hazır!"
lvs "$VG"
echo ""
echo "Testleri çalıştırmak için:"
echo "    ROLLBACKX_LVM_VG=$VG ROLLBACKX_LVM_ROOT=$ROOT_LV \\"
echo "    cargo test --test lvm_snapshot_test -- --nocapture"
echo ""
echo "Temizlik için:"
echo "    vgremove -f $VG && losetup -d $LOOP && rm $IMG"
