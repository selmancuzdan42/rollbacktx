//! Snapshot backend trait'i ve implementasyonları.
//!
//! Her dosya sistemi için ayrı bir backend modülü mevcuttur.
//! `detect_backend()` fonksiyonu sisteme uygun backend'i otomatik seçer.

pub mod btrfs;
pub mod lvm;
pub mod rsync;

pub use btrfs::BtrfsBackend;
pub use lvm::LvmThinBackend;
pub use rsync::RsyncBackend;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, RollbackError};
use crate::snapshot::{Snapshot, SnapshotTrigger};

/// Geri yükleme işleminin tamamlanması için gereken koşul.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreRequirement {
    /// Geri yükleme reboot sonrası etkin olur.
    Reboot,
    /// Geri yükleme anlık uygulanır (nadiren mümkün).
    Immediate,
}

/// Tek bir doğrulama kontrolünün sonucu.
#[derive(Debug, Clone)]
pub struct CheckSonucu {
    /// Kontrol edilen öğenin adı (örn: "etc/passwd").
    pub ad: String,
    /// Kontrol başarılı mı?
    pub basarili: bool,
    /// Detay mesajı.
    pub mesaj: String,
}

/// Snapshot doğrulama raporu — birden fazla `CheckSonucu` içerir.
#[derive(Debug, Clone)]
pub struct DogrulamaRaporu {
    /// Tüm kontroller başarılı mı?
    pub tumu_basarili: bool,
    /// Yapılan kontrollerin listesi.
    pub kontroller: Vec<CheckSonucu>,
}

/// Dosya sistemi snapshot backend'i trait'i.
///
/// Her backend (Btrfs, LVM Thin, Rsync) bu trait'i implemente eder.
/// `Send + Sync` gereksinimi, backend'lerin thread'ler arası paylaşılabilmesini sağlar.
pub trait SnapshotBackend: Send + Sync {
    /// Backend'in adını döndürür (örn: "Btrfs", "Rsync").
    fn name(&self) -> &str;

    /// Backend'in bu sistemde kullanılabilir olup olmadığını kontrol eder.
    fn is_available(&self) -> Result<bool>;

    /// Yeni bir snapshot oluşturur ve `Snapshot` kaydını döndürür.
    fn create(&self, name: &str, desc: Option<&str>, trigger: SnapshotTrigger, next_id: u32)
        -> Result<Snapshot>;

    /// Snapshot'ı geri yükler. Genellikle reboot gerektirir.
    fn restore(&self, snapshot: &Snapshot) -> Result<RestoreRequirement>;

    /// Snapshot'ı dosya sisteminden siler.
    fn delete(&self, snapshot: &Snapshot) -> Result<()>;

    /// Snapshot'ın güncel disk kullanımını yeniden hesaplar.
    fn refresh_size(&self, snapshot: &Snapshot) -> Option<u64>;

    /// Snapshot'ı belirtilen noktaya salt-okunur olarak bağlar (mount).
    fn mount(&self, snapshot: &Snapshot, mount_point: &Path) -> Result<()>;

    /// Snapshot'ın bütünlüğünü doğrular (kritik dosya/dizin kontrolü).
    fn verify(&self, snapshot: &Snapshot) -> Result<DogrulamaRaporu>;
}

// ── Doğrulama yardımcısı ────────────────────────────────────────────────────

/// Bağlı bir snapshot dizininde kritik sistem dosyalarını kontrol eder.
/// `base` = snapshot'ın kök dizini (örn: /var/lib/rollbackx/snapshots/1_test_…/)
pub fn verify_snapshot_dir(base: &Path) -> DogrulamaRaporu {
    let mut kontroller = Vec::new();

    // Kritik dizinler
    for (ad, rel) in [
        ("etc/ dizini", "etc"),
        ("usr/ dizini", "usr"),
        ("boot/ dizini", "boot"),
    ] {
        let path = base.join(rel);
        let basarili = path.is_dir();
        kontroller.push(CheckSonucu {
            ad: ad.to_string(),
            basarili,
            mesaj: if basarili {
                "Mevcut".to_string()
            } else {
                format!("{rel}/ dizini bulunamadı")
            },
        });
    }

    // Kritik dosyalar
    for (ad, rel) in [
        ("etc/passwd", "etc/passwd"),
        ("etc/fstab", "etc/fstab"),
        ("etc/os-release", "etc/os-release"),
    ] {
        let path = base.join(rel);
        let basarili = path.is_file();
        kontroller.push(CheckSonucu {
            ad: ad.to_string(),
            basarili,
            mesaj: if basarili {
                "Mevcut".to_string()
            } else {
                format!("{rel} dosyası bulunamadı")
            },
        });
    }

    // etc/passwd geçerlilik: root satırı var mı?
    let passwd_path = base.join("etc/passwd");
    if passwd_path.is_file() {
        let icerik = std::fs::read_to_string(&passwd_path).unwrap_or_default();
        let gecerli = icerik.lines().any(|l| l.starts_with("root:"));
        kontroller.push(CheckSonucu {
            ad: "etc/passwd root kaydı".to_string(),
            basarili: gecerli,
            mesaj: if gecerli {
                "root kaydı mevcut".to_string()
            } else {
                "root kaydı eksik — passwd bozuk olabilir".to_string()
            },
        });
    }

    // usr/bin/ dizini (modern sistemlerde /bin → /usr/bin symlink olabilir)
    let usrbin = base.join("usr/bin");
    let basarili = usrbin.is_dir();
    kontroller.push(CheckSonucu {
        ad: "usr/bin/ dizini".to_string(),
        basarili,
        mesaj: if basarili {
            "Mevcut".to_string()
        } else {
            "usr/bin/ bulunamadı".to_string()
        },
    });

    let tumu_basarili = kontroller.iter().all(|c| c.basarili);
    DogrulamaRaporu { tumu_basarili, kontroller }
}

// ── Mount yardımcıları (backend-agnostik) ────────────────────────────────────

/// Snapshot için sabit mount noktası: /mnt/rollbackx/{id}/
pub fn snapshot_mount_point(id: u32) -> PathBuf {
    PathBuf::from(format!("/mnt/rollbackx/{id}"))
}

/// Belirtilen dizinin aktif bir mount noktası olup olmadığını kontrol eder.
///
/// `mountpoint -q` kullanır. `findmnt --target` yerine tercih edilir çünkü
/// `findmnt` üst dizinlerdeki mount'ları da döndürür.
pub fn is_snapshot_mounted(mount_point: &Path) -> bool {
    Command::new("mountpoint")
        .arg("-q")
        .arg(mount_point)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Snapshot mount noktasını ayırır ve dizini temizler.
///
/// Bağlı değilse `umount` atlanır, dizin yine de silinir (stale mount point temizliği).
pub fn unmount_snapshot(mount_point: &Path) -> Result<()> {
    let mp = mount_point.to_string_lossy().to_string();

    if is_snapshot_mounted(mount_point) {
        let out = Command::new("umount").arg(&mp).output().map_err(|e| {
            RollbackError::CommandFailed {
                cmd: format!("umount {mp}"),
                reason: e.to_string(),
            }
        })?;
        if !out.status.success() {
            return Err(RollbackError::CommandFailed {
                cmd: format!("umount {mp}"),
                reason: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
    }

    // Dizini her durumda temizle (stale mount point kalmasın)
    let _ = std::fs::remove_dir(mount_point);
    Ok(())
}

/// `/mnt/rollbackx/` altındaki tüm stale dizinleri temizle (reboot sonrası).
pub fn cleanup_stale_mount_points() {
    let base = std::path::Path::new("/mnt/rollbackx");
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_snapshot_mounted(&path) {
                let _ = std::fs::remove_dir(&path);
            }
        }
    }
}

/// Sistemdeki uygun backend'i otomatik seçer.
///
/// Öncelik sırası: Btrfs → LVM Thin → Rsync (evrensel fallback).
/// Hiçbiri kullanılamıyorsa `NoCompatibleBackend` hatası döner.
pub fn detect_backend() -> Result<Box<dyn SnapshotBackend>> {
    let btrfs = BtrfsBackend::new();
    if btrfs.is_available()? {
        return Ok(Box::new(btrfs));
    }

    let lvm = LvmThinBackend::new();
    if lvm.is_available()? {
        return Ok(Box::new(lvm));
    }

    // Rsync: ext4 ve diğer tüm dosya sistemleri için evrensel fallback
    let rsync = RsyncBackend::new();
    if rsync.is_available()? {
        return Ok(Box::new(rsync));
    }

    Err(RollbackError::NoCompatibleBackend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_snapshot_dir_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        // Kritik dizinleri oluştur
        std::fs::create_dir_all(base.join("etc")).unwrap();
        std::fs::create_dir_all(base.join("usr/bin")).unwrap();
        std::fs::create_dir_all(base.join("boot")).unwrap();

        // Kritik dosyaları oluştur
        std::fs::write(base.join("etc/passwd"), "root:x:0:0:root:/root:/bin/bash\n").unwrap();
        std::fs::write(base.join("etc/fstab"), "# fstab\n").unwrap();
        std::fs::write(base.join("etc/os-release"), "ID=pardus\n").unwrap();

        let rapor = verify_snapshot_dir(base);
        assert!(rapor.tumu_basarili);
        assert!(rapor.kontroller.len() >= 7);
        assert!(rapor.kontroller.iter().all(|c| c.basarili));
    }

    #[test]
    fn test_verify_snapshot_dir_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        // Boş dizin — hiçbir şey yok
        let rapor = verify_snapshot_dir(base);
        assert!(!rapor.tumu_basarili);
        assert!(rapor.kontroller.iter().any(|c| !c.basarili));
    }

    #[test]
    fn test_verify_passwd_no_root() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();

        std::fs::create_dir_all(base.join("etc")).unwrap();
        std::fs::create_dir_all(base.join("usr/bin")).unwrap();
        std::fs::create_dir_all(base.join("boot")).unwrap();
        std::fs::write(base.join("etc/passwd"), "nobody:x:65534:65534:nobody:/:\n").unwrap();
        std::fs::write(base.join("etc/fstab"), "").unwrap();
        std::fs::write(base.join("etc/os-release"), "").unwrap();

        let rapor = verify_snapshot_dir(base);
        assert!(!rapor.tumu_basarili);
        let root_check = rapor.kontroller.iter().find(|c| c.ad.contains("root")).unwrap();
        assert!(!root_check.basarili);
    }

    #[test]
    fn test_detect_backend_runs() {
        // detect_backend panik atmamalı — sonuç Ok veya Err olabilir
        let result = detect_backend();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_snapshot_mount_point_format() {
        let mp = snapshot_mount_point(42);
        assert_eq!(mp, PathBuf::from("/mnt/rollbackx/42"));
    }
}
