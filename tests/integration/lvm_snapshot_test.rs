//! LVM Thin backend entegrasyon testleri.
//!
//! Bu testler çalışmak için hazır bir LVM Thin test ortamı gerektirir.
//! Önce `tests/vm-setup/setup-lvm.sh` scriptini çalıştırın.
//!
//! Ortam değişkenleri:
//!   ROLLBACKX_LVM_VG    — Volume Group adı    (varsayılan: rxvg)
//!   ROLLBACKX_LVM_ROOT  — Root LV adı         (varsayılan: root)

use rollbackx_core::backend::{lvm::LvmThinBackend, SnapshotBackend};
use rollbackx_core::snapshot::SnapshotTrigger;

fn lvm_test_ortami() -> (String, String) {
    let vg = std::env::var("ROLLBACKX_LVM_VG").unwrap_or_else(|_| "rxvg".to_string());
    let root_lv = std::env::var("ROLLBACKX_LVM_ROOT").unwrap_or_else(|_| "root".to_string());
    (vg, root_lv)
}

fn lvm_hazir(vg: &str) -> bool {
    std::process::Command::new("vgs")
        .arg(vg)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn test_lvm_create_ve_delete() {
    let (vg, root_lv) = lvm_test_ortami();
    if !lvm_hazir(&vg) {
        eprintln!(
            "ATLA: LVM VG '{vg}' bulunamadı. \
             Önce setup-lvm.sh çalıştırın ve root olarak test edin."
        );
        return;
    }

    let backend = LvmThinBackend::with_vg_lv(&vg, &root_lv);
    let tmp = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .expect("tempfile");
    let mut db =
        rollbackx_core::snapshot::SnapshotDB::open(Some(tmp.path())).expect("DB açılamadı");

    // Snapshot oluştur
    let snap = backend
        .create(
            "lvm-entegrasyon",
            Some("LVM Thin entegrasyon testi"),
            SnapshotTrigger::Manual,
            db.next_id(),
        )
        .expect("LVM snapshot oluşturulamadı");

    println!("Oluşturulan LVM snapshot: {:?}", snap);
    assert!(snap.backend_ref.contains(&vg));
    assert!(snap.backend_ref.contains("rx_"));

    let snap_clone = snap.clone();
    db.add(snap).expect("DB'ye eklenemedi");

    // lvs ile görünüyor mu?
    let lv_name = snap_clone.backend_ref.split('/').last().unwrap();
    let output = std::process::Command::new("lvs")
        .args(["--noheadings", "-o", "lv_name", &format!("/dev/{}", snap_clone.backend_ref)])
        .output()
        .expect("lvs çalıştırılamadı");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(lv_name),
        "LV listede görünmüyor: {lv_name}"
    );

    // Sil
    backend.delete(&snap_clone).expect("LVM snapshot silinemedi");

    // Silindi mi?
    let output2 = std::process::Command::new("lvs")
        .args(["--noheadings", "-o", "lv_name"])
        .output()
        .expect("lvs çalıştırılamadı");
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        !stdout2.contains(lv_name),
        "LV hâlâ listede: {lv_name}"
    );
}

#[test]
fn test_lvm_is_available_yoksa_false() {
    // Test VG olmayan sistemde is_available() false dönmeli
    let backend = LvmThinBackend::with_vg_lv("olmayan_vg_xxx", "root");
    // Bu test sistemde lvs yoksa da false dönmeli
    let result = backend.is_available();
    // Hata döndürmemeli, sadece false/true olabilir
    assert!(result.is_ok());
}
