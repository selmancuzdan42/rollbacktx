//! # RollbackX Core
//!
//! Pardus Linux için Btrfs, LVM Thin ve Rsync tabanlı sistem snapshot yöneticisi.
//!
//! Bu crate, snapshot oluşturma, geri yükleme, silme, dışa/içe aktarma ve zamanlama
//! işlemlerinin tüm iş mantığını barındırır. `SnapshotBackend` trait'i üzerinden
//! farklı dosya sistemi backend'leri desteklenir:
//!
//! - **Btrfs** — Btrfs subvolume snapshot'ları (salt-okunur, CoW)
//! - **LVM Thin** — LVM thin provisioning snapshot'ları
//! - **Rsync** — Hard-link tabanlı artımlı yedekleme (ext4 vb. için evrensel fallback)
//!
//! ## Kullanım
//!
//! ```rust,no_run
//! use rollbackx_core::backend::detect_backend;
//! use rollbackx_core::snapshot::{SnapshotDB, SnapshotTrigger};
//!
//! let backend = detect_backend().expect("Backend bulunamadı");
//! let mut db = SnapshotDB::open(None).expect("DB açılamadı");
//! let snap = backend.create("yedek", None, SnapshotTrigger::Manual, db.next_id())
//!     .expect("Snapshot oluşturulamadı");
//! db.add(snap).expect("DB'ye eklenemedi");
//! ```

pub mod backend;
pub mod error;
pub mod export;
pub mod schedule;
pub mod snapshot;

/// Snapshot veritabanı dosyasının varsayılan yolu.
pub const DB_PATH: &str = "/var/lib/rollbackx/snapshots.json";

pub use error::RollbackError;
pub use export::{export, import, inspect, RxSnapMeta};
pub use schedule::Schedule;
pub use snapshot::{BackendKind, Snapshot, SnapshotDB, SnapshotTrigger};
