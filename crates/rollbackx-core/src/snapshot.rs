//! Snapshot veri modeli ve JSON veritabanı (CRUD).

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Result, RollbackError};

/// Snapshot'ın hangi dosya sistemi backend'i ile oluşturulduğunu belirtir.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BackendKind {
    /// Btrfs subvolume snapshot (CoW, salt-okunur)
    Btrfs,
    /// LVM thin provisioning snapshot
    LvmThin,
    /// Rsync hard-link tabanlı artımlı yedekleme
    Rsync,
}

impl std::fmt::Display for BackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendKind::Btrfs   => write!(f, "Btrfs"),
            BackendKind::LvmThin => write!(f, "LVM Thin"),
            BackendKind::Rsync   => write!(f, "Rsync"),
        }
    }
}

/// Snapshot'ın oluşturulma nedenini belirtir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SnapshotTrigger {
    /// Kullanıcı tarafından elle oluşturuldu.
    Manual,
    /// APT paket işlemi öncesi otomatik oluşturuldu.
    Apt {
        /// İşlem yapılan paketlerin listesi.
        packages: Vec<String>,
    },
    /// Zamanlayıcı (systemd timer) tarafından otomatik oluşturuldu.
    Cron,
}

impl std::fmt::Display for SnapshotTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotTrigger::Manual => write!(f, "Manuel"),
            SnapshotTrigger::Apt { packages } => {
                if packages.is_empty() {
                    write!(f, "APT")
                } else {
                    write!(f, "APT ({})", packages.join(", "))
                }
            }
            SnapshotTrigger::Cron => write!(f, "Zamanlayıcı"),
        }
    }
}

/// Tek bir sistem snapshot kaydı.
///
/// JSON olarak `/var/lib/rollbackx/snapshots.json` dosyasında saklanır.
/// `locked` alanı `#[serde(default)]` ile geriye dönük uyumludur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Benzersiz snapshot kimliği (otomatik artan).
    pub id: u32,
    /// Kullanıcı tarafından verilen kısa ad.
    pub name: String,
    /// İsteğe bağlı açıklama.
    pub description: Option<String>,
    /// Oluşturulma zaman damgası.
    pub created_at: DateTime<Local>,
    /// Oluşturulma tetikleyicisi (manuel, APT, zamanlayıcı).
    pub trigger: SnapshotTrigger,
    /// Kullanılan dosya sistemi backend'i.
    pub backend: BackendKind,
    /// Backend'e özgü referans (Btrfs: subvolume adı, LVM: "vg/lv", Rsync: dizin adı).
    pub backend_ref: String,
    /// Tahmini disk kullanımı (bayt).
    pub size_bytes: Option<u64>,
    /// Kilitliyse silinemez ve geri yüklenemez.
    #[serde(default)]
    pub locked: bool,
}

impl Snapshot {
    /// Boyutu insan okunabilir formata çevirir (B, KB, MB, GB).
    pub fn size_human(&self) -> String {
        match self.size_bytes {
            None => "—".to_string(),
            Some(b) if b < 1024 => format!("{b} B"),
            Some(b) if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
            Some(b) if b < 1024 * 1024 * 1024 => {
                format!("{:.1} MB", b as f64 / (1024.0 * 1024.0))
            }
            Some(b) => format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
        }
    }
}

/// Snapshot veritabanı — JSON dosyası üzerinde CRUD işlemleri sağlar.
///
/// Varsayılan dosya yolu: `/var/lib/rollbackx/snapshots.json`
pub struct SnapshotDB {
    /// Veritabanı dosyasının yolu.
    path: PathBuf,
    /// Bellekteki snapshot listesi.
    snapshots: Vec<Snapshot>,
    /// Bir sonraki snapshot için atanacak ID.
    next_id: u32,
}

impl SnapshotDB {
    /// Veritabanını belirtilen yoldan açar.
    ///
    /// - `path = None` ise varsayılan yol (`DB_PATH`) kullanılır.
    /// - Dosya yoksa boş veritabanı oluşturur.
    /// - Dosya boşsa boş liste döndürür (bozuk JSON hatası vermez).
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let path = path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(crate::DB_PATH));

        if path.exists() {
            let data = std::fs::read_to_string(&path)?;
            if data.trim().is_empty() {
                return Ok(Self { path, snapshots: Vec::new(), next_id: 1 });
            }
            let snapshots: Vec<Snapshot> = serde_json::from_str(&data)?;
            let next_id = snapshots.iter().map(|s| s.id).max().unwrap_or(0) + 1;
            Ok(Self { path, snapshots, next_id })
        } else {
            // Dizin yoksa oluştur
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Ok(Self { path, snapshots: Vec::new(), next_id: 1 })
        }
    }

    /// Veritabanını diske yazar (JSON pretty-print).
    pub fn save(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.snapshots)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }

    /// Bir sonraki snapshot için kullanılacak ID'yi döndürür.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Yeni bir snapshot ekler ve veritabanını kaydeder.
    pub fn add(&mut self, snapshot: Snapshot) -> Result<()> {
        self.next_id = snapshot.id + 1;
        self.snapshots.push(snapshot);
        self.save()
    }

    /// Belirtilen ID'ye sahip snapshot'ı siler ve döndürür.
    ///
    /// Snapshot bulunamazsa `SnapshotNotFound` hatası verir.
    pub fn remove(&mut self, id: u32) -> Result<Snapshot> {
        let pos = self
            .snapshots
            .iter()
            .position(|s| s.id == id)
            .ok_or(RollbackError::SnapshotNotFound(id))?;
        let snap = self.snapshots.remove(pos);
        self.save()?;
        Ok(snap)
    }

    /// Belirtilen ID'ye sahip snapshot'a salt-okunur referans döndürür.
    pub fn get(&self, id: u32) -> Result<&Snapshot> {
        self.snapshots
            .iter()
            .find(|s| s.id == id)
            .ok_or(RollbackError::SnapshotNotFound(id))
    }

    /// Belirtilen ID'ye sahip snapshot'a değiştirilebilir referans döndürür.
    pub fn get_mut(&mut self, id: u32) -> Result<&mut Snapshot> {
        self.snapshots
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(RollbackError::SnapshotNotFound(id))
    }

    /// Snapshot'ın kilit durumunu değiştirir.
    ///
    /// Kilitli snapshot'lar silinemez ve geri yüklenemez.
    pub fn set_lock(&mut self, id: u32, locked: bool) -> Result<()> {
        self.get_mut(id)?.locked = locked;
        self.save()
    }

    /// Snapshot'ın adını ve açıklamasını günceller.
    pub fn rename(&mut self, id: u32, new_name: String, new_desc: Option<String>) -> Result<()> {
        let snap = self.get_mut(id)?;
        snap.name = new_name;
        snap.description = new_desc;
        self.save()
    }

    /// Tüm snapshot'ları döndürür.
    pub fn all(&self) -> &[Snapshot] {
        &self.snapshots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_db_open_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent.json");
        let db = SnapshotDB::open(Some(&path)).unwrap();
        assert_eq!(db.all().len(), 0);
        assert_eq!(db.next_id(), 1);
    }

    #[test]
    fn test_db_open_empty_file() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "").unwrap();
        let db = SnapshotDB::open(Some(tmp.path())).unwrap();
        assert_eq!(db.all().len(), 0);
    }

    #[test]
    fn test_db_add_remove_get() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        let mut db = SnapshotDB::open(Some(&path)).unwrap();

        let snap = Snapshot {
            id: 1,
            name: "test".to_string(),
            description: Some("açıklama".to_string()),
            created_at: Local::now(),
            trigger: SnapshotTrigger::Manual,
            backend: BackendKind::Rsync,
            backend_ref: "1_test_20260101".to_string(),
            size_bytes: Some(1024),
            locked: false,
        };
        db.add(snap).unwrap();
        assert_eq!(db.all().len(), 1);
        assert_eq!(db.next_id(), 2);

        let fetched = db.get(1).unwrap();
        assert_eq!(fetched.name, "test");

        let removed = db.remove(1).unwrap();
        assert_eq!(removed.id, 1);
        assert_eq!(db.all().len(), 0);
    }

    #[test]
    fn test_db_set_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("lock.json");
        let mut db = SnapshotDB::open(Some(&path)).unwrap();

        let snap = Snapshot {
            id: 1,
            name: "locktest".to_string(),
            description: None,
            created_at: Local::now(),
            trigger: SnapshotTrigger::Manual,
            backend: BackendKind::Rsync,
            backend_ref: "1_locktest".to_string(),
            size_bytes: None,
            locked: false,
        };
        db.add(snap).unwrap();
        assert!(!db.get(1).unwrap().locked);

        db.set_lock(1, true).unwrap();
        assert!(db.get(1).unwrap().locked);

        db.set_lock(1, false).unwrap();
        assert!(!db.get(1).unwrap().locked);
    }

    #[test]
    fn test_db_get_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("empty.json");
        let db = SnapshotDB::open(Some(&path)).unwrap();
        assert!(db.get(999).is_err());
    }

    #[test]
    fn test_db_persistence() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("persist.json");

        {
            let mut db = SnapshotDB::open(Some(&path)).unwrap();
            let snap = Snapshot {
                id: 1,
                name: "persist".to_string(),
                description: None,
                created_at: Local::now(),
                trigger: SnapshotTrigger::Cron,
                backend: BackendKind::Btrfs,
                backend_ref: "@1_persist".to_string(),
                size_bytes: Some(2048),
                locked: false,
            };
            db.add(snap).unwrap();
        }

        // Dosyadan tekrar aç
        let db2 = SnapshotDB::open(Some(&path)).unwrap();
        assert_eq!(db2.all().len(), 1);
        assert_eq!(db2.get(1).unwrap().name, "persist");
        assert_eq!(db2.next_id(), 2);
    }

    #[test]
    fn test_size_human() {
        let snap = Snapshot {
            id: 1, name: "s".into(), description: None,
            created_at: Local::now(), trigger: SnapshotTrigger::Manual,
            backend: BackendKind::Rsync, backend_ref: "ref".into(),
            size_bytes: None, locked: false,
        };
        assert_eq!(snap.size_human(), "—");

        let snap2 = Snapshot { size_bytes: Some(512), ..snap.clone() };
        assert_eq!(snap2.size_human(), "512 B");

        let snap3 = Snapshot { size_bytes: Some(1024 * 1024 * 1024 + 512 * 1024 * 1024), ..snap };
        assert!(snap3.size_human().contains("GB"));
    }

    #[test]
    fn test_backend_kind_display() {
        assert_eq!(BackendKind::Btrfs.to_string(), "Btrfs");
        assert_eq!(BackendKind::LvmThin.to_string(), "LVM Thin");
        assert_eq!(BackendKind::Rsync.to_string(), "Rsync");
    }

    #[test]
    fn test_snapshot_trigger_display() {
        assert_eq!(SnapshotTrigger::Manual.to_string(), "Manuel");
        assert_eq!(SnapshotTrigger::Cron.to_string(), "Zamanlayıcı");
        let apt = SnapshotTrigger::Apt { packages: vec!["vim".into(), "git".into()] };
        assert!(apt.to_string().contains("vim"));
    }
}
