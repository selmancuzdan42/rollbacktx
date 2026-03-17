//! Btrfs backend entegrasyon testleri.
//!
//! Bu testler çalışmak için hazır bir Btrfs test ortamı gerektirir.
//! Önce `tests/vm-setup/setup-btrfs.sh` scriptini çalıştırın.
//!
//! Ortam değişkenleri:
//!   ROLLBACKX_BTRFS_ROOT    — Btrfs kök mount noktası (varsayılan: /mnt/rx-btrfs-test)
//!   ROLLBACKX_SNAP_DIR      — Snapshot dizini       (varsayılan: /mnt/rx-btrfs-test/.snapshots)

use rollbackx_core::backend::{btrfs::BtrfsBackend, SnapshotBackend};
use rollbackx_core::snapshot::SnapshotTrigger;

fn btrfs_test_ortami() -> (String, String) {
    let root = std::env::var("ROLLBACKX_BTRFS_ROOT")
        .unwrap_or_else(|_| "/mnt/rx-btrfs-test".to_string());
    let snap_dir = std::env::var("ROLLBACKX_SNAP_DIR")
        .unwrap_or_else(|_| format!("{root}/.snapshots"));
    (root, snap_dir)
}

fn backend_hazir(root: &str, snap_dir: &str) -> bool {
    std::path::Path::new(snap_dir).exists()
        && std::path::Path::new(root).exists()
}

#[test]
fn test_btrfs_create_ve_delete() {
    let (root, snap_dir) = btrfs_test_ortami();
    if !backend_hazir(&root, &snap_dir) {
        eprintln!(
            "ATLA: Btrfs test ortamı bulunamadı ({snap_dir}). \
             Önce setup-btrfs.sh çalıştırın."
        );
        return;
    }

    let backend = BtrfsBackend::with_paths(&root, &snap_dir);

    // Geçici DB
    let tmp_db = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .expect("tempfile oluşturulamadı");
    let db_path = tmp_db.path();
    let mut db = rollbackx_core::snapshot::SnapshotDB::open(Some(db_path))
        .expect("DB açılamadı");

    // Snapshot oluştur
    let snap = backend
        .create(
            "entegrasyon-testi",
            Some("Otomatik entegrasyon testi"),
            SnapshotTrigger::Manual,
            db.next_id(),
        )
        .expect("Snapshot oluşturulamadı");

    println!("Oluşturulan snapshot: {:?}", snap);
    assert_eq!(snap.name, "entegrasyon-testi");
    assert!(!snap.backend_ref.is_empty());

    let snap_clone = snap.clone();
    db.add(snap).expect("DB'ye eklenemedi");

    // Snapshot dizininde görünüyor mu?
    let snap_yol = format!("{snap_dir}/{}", snap_clone.backend_ref);
    assert!(
        std::path::Path::new(&snap_yol).exists(),
        "Snapshot dizini oluşturulmadı: {snap_yol}"
    );

    // Sil
    backend
        .delete(&snap_clone)
        .expect("Snapshot silinemedi");

    assert!(
        !std::path::Path::new(&snap_yol).exists(),
        "Snapshot dizini hâlâ var: {snap_yol}"
    );
}

#[test]
fn test_btrfs_list_bos_dizin() {
    let (_root, snap_dir) = btrfs_test_ortami();
    if !backend_hazir(&_root, &snap_dir) {
        return;
    }
    // Boş (temizlenmiş) dizinde liste boş dönmeli — DB bağımsız kontrol
    // (sadece dizin varlığı test ediliyor)
    assert!(std::path::Path::new(&snap_dir).is_dir());
}

#[test]
fn test_snapshotdb_ekle_cikar() {
    let tmp = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .expect("tempfile");

    let mut db = rollbackx_core::snapshot::SnapshotDB::open(Some(tmp.path()))
        .expect("DB açılamadı");

    // Gerçek backend olmadan sahte snapshot ekle
    let sahte = rollbackx_core::snapshot::Snapshot {
        id: db.next_id(),
        name: "test".to_string(),
        description: None,
        created_at: chrono::Local::now(),
        trigger: SnapshotTrigger::Manual,
        backend: rollbackx_core::snapshot::BackendKind::Btrfs,
        backend_ref: "@1_test_20260101_000000".to_string(),
        size_bytes: Some(1024),
        locked: false,
    };

    let id = sahte.id;
    db.add(sahte).unwrap();
    assert_eq!(db.all().len(), 1);

    let alinan = db.get(id).unwrap();
    assert_eq!(alinan.name, "test");

    db.remove(id).unwrap();
    assert_eq!(db.all().len(), 0);
}
