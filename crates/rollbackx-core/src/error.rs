//! Hata tipleri ve `Result` type alias.

use thiserror::Error;

/// RollbackX işlemlerinde oluşabilecek hata türleri.
///
/// Her variant, hatanın kaynağına göre yapılandırılmıştır.
/// `thiserror` crate'i ile `Display` ve `Error` trait'leri otomatik türetilir.
#[derive(Debug, Error)]
pub enum RollbackError {
    /// Sistemde desteklenen hiçbir backend bulunamadığında döner.
    /// Btrfs, LVM Thin veya Rsync'ten en az biri mevcut olmalıdır.
    #[error("Uyumlu bir backend bulunamadı (Btrfs veya LVM Thin gerekli)")]
    NoCompatibleBackend,

    /// Seçilen backend mevcut ancak kullanıma hazır değil.
    #[error("Backend hazır değil: {0}")]
    BackendNotAvailable(String),

    /// Verilen ID ile eşleşen snapshot veritabanında bulunamadı.
    #[error("Snapshot bulunamadı: id={0}")]
    SnapshotNotFound(u32),

    /// JSON veritabanı okuma/yazma sırasında oluşan yapısal hatalar.
    #[error("Snapshot veritabanı hatası: {0}")]
    DatabaseError(String),

    /// Harici bir komut (btrfs, rsync, lvcreate vb.) başarısız olduğunda döner.
    #[error("Komut çalıştırma hatası: {cmd} → {reason}")]
    CommandFailed {
        /// Çalıştırılan komut satırı
        cmd: String,
        /// Başarısızlık nedeni (genellikle stderr çıktısı)
        reason: String,
    },

    /// İşlem root (UID 0) yetkisi gerektirdiğinde ve mevcut kullanıcı root değilse döner.
    #[error("Yetki hatası: Bu işlem için root (UID 0) gereklidir")]
    PermissionDenied,

    /// Dosya sistemi I/O hatası.
    #[error("I/O hatası: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serileştirme/ayrıştırma hatası.
    #[error("JSON parse hatası: {0}")]
    Json(#[from] serde_json::Error),

    /// Kullanıcı girdisi geçersiz olduğunda döner (dosya yolu, parametre vb.).
    #[error("Geçersiz giriş: {0}")]
    InvalidInput(String),
}

/// RollbackX işlemleri için kısayol `Result` tipi.
pub type Result<T> = std::result::Result<T, RollbackError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display_messages() {
        let e = RollbackError::NoCompatibleBackend;
        assert!(e.to_string().contains("backend"));

        let e = RollbackError::SnapshotNotFound(42);
        assert!(e.to_string().contains("42"));

        let e = RollbackError::CommandFailed {
            cmd: "btrfs snapshot".to_string(),
            reason: "permission denied".to_string(),
        };
        let msg = e.to_string();
        assert!(msg.contains("btrfs snapshot"));
        assert!(msg.contains("permission denied"));

        let e = RollbackError::PermissionDenied;
        assert!(e.to_string().contains("root"));

        let e = RollbackError::InvalidInput("boş ad".to_string());
        assert!(e.to_string().contains("boş ad"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "dosya yok");
        let rb_err: RollbackError = io_err.into();
        assert!(rb_err.to_string().contains("dosya yok"));
    }
}
