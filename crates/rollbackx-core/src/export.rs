/// RollbackX .rxsnap export/import modülü
///
/// Format: zstd sıkıştırmalı tar arşivi
/// İçerik:
///   metadata.json   → snapshot bilgileri
///   data/           → snapshot dosyaları (Rsync backend)
///   data.btrfs.zst  → btrfs send stream (Btrfs backend)
///   checksum.sha256 → SHA256 bütünlük kontrolü (metadata + stream)
///
/// Disk tasarrufu ilkesi:
///   Export — Rsync için data/ hiçbir zaman /tmp'ye kopyalanmaz; tar --transform
///             ile doğrudan kaynaktan arşivlenir.
///   Import — Rsync için arşiv doğrudan hedefe açılır (--strip-components=1);
///             Btrfs için data.btrfs.zst stdout'a çıkarılıp pipe ile btrfs receive'e gönderilir.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::error::{Result, RollbackError};
use crate::snapshot::{BackendKind, Snapshot};

const FORMAT_VERSION: u32 = 1;

const RSYNC_SNAPSHOT_BASE: &str = "/var/lib/rollbackx/snapshots";
const BTRFS_SNAPSHOT_BASE: &str  = "/.snapshots";

/// `.rxsnap` arşivi içindeki metadata bilgileri.
///
/// `metadata.json` dosyası olarak arşivin içinde saklanır.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RxSnapMeta {
    /// RollbackX sürümü.
    pub rx_version: String,
    /// Arşiv format sürümü.
    pub format_version: u32,
    /// Snapshot'ın orijinal ID'si.
    pub snapshot_id: u32,
    /// Snapshot adı.
    pub name: String,
    /// İsteğe bağlı açıklama.
    pub description: Option<String>,
    /// Oluşturulma zamanı (RFC 3339).
    pub created_at: String,
    /// Kullanılan backend türü.
    pub backend: BackendKind,
    /// Dışa aktarıldığı makinenin hostname'i.
    pub hostname: String,
    /// Dışa aktarılma zamanı (RFC 3339).
    pub exported_at: String,
}

// ── Export ────────────────────────────────────────────────────────────────

/// Snapshot'ı `.rxsnap` dosyasına dışa aktar.
pub fn export(snapshot: &Snapshot, output: &Path) -> Result<()> {
    let tmp = tempfile::TempDir::new()?;
    let tmp_path = tmp.path();

    let meta = RxSnapMeta {
        rx_version:     env!("CARGO_PKG_VERSION").to_string(),
        format_version: FORMAT_VERSION,
        snapshot_id:    snapshot.id,
        name:           snapshot.name.clone(),
        description:    snapshot.description.clone(),
        created_at:     snapshot.created_at.to_rfc3339(),
        backend:        snapshot.backend.clone(),
        hostname:       read_hostname(),
        exported_at:    Local::now().to_rfc3339(),
    };
    fs::write(
        tmp_path.join("metadata.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;

    match &snapshot.backend {
        BackendKind::Rsync => {
            // Rsync: /tmp'ye kopyalama yok — doğrudan kaynaktan arşivle
            let src = resolve_rsync_path(&snapshot.backend_ref)?;
            let checksum = compute_checksum(tmp_path)?;
            fs::write(tmp_path.join("checksum.sha256"), &checksum)?;
            pack_rsync(&src, tmp_path, output)?;
        }
        BackendKind::Btrfs => {
            export_btrfs(&snapshot.backend_ref, tmp_path)?;
            let checksum = compute_checksum(tmp_path)?;
            fs::write(tmp_path.join("checksum.sha256"), &checksum)?;
            pack(tmp_path, output)?;
        }
        BackendKind::LvmThin => {
            return Err(RollbackError::InvalidInput(
                "LVM Thin backend için export henüz desteklenmiyor".to_string(),
            ))
        }
    }

    Ok(())
}

// ── Import ────────────────────────────────────────────────────────────────

/// `.rxsnap` dosyasını içe aktar.
/// Döndürür: `(meta, dest_path)` — `dest_path` oluşturulan snapshot dizini.
pub fn import(rxsnap_path: &Path, target_base: &Path) -> Result<(RxSnapMeta, PathBuf)> {
    if !rxsnap_path.exists() {
        return Err(RollbackError::InvalidInput(format!(
            "Dosya bulunamadı: {}",
            rxsnap_path.display()
        )));
    }

    // Sadece metadata.json + checksum.sha256 için küçük bir tmp dir
    let tmp = tempfile::TempDir::new()?;
    let tmp_path = tmp.path();

    // Arşivden sadece küçük dosyaları çıkar
    // Not: tar -C dir . ile oluşturulan arşivde girişler "./metadata.json" şeklinde saklanır
    extract_named(rxsnap_path, tmp_path, &["./metadata.json", "./checksum.sha256"])?;

    let meta_path = tmp_path.join("metadata.json");
    if !meta_path.exists() {
        return Err(RollbackError::InvalidInput(
            "Geçersiz .rxsnap: metadata.json bulunamadı".to_string(),
        ));
    }
    let meta: RxSnapMeta = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;

    if meta.format_version > FORMAT_VERSION {
        return Err(RollbackError::InvalidInput(format!(
            "Bu dosya daha yeni bir RollbackX sürümü gerektiriyor (format v{})",
            meta.format_version
        )));
    }

    // Checksum doğrula (sadece metadata.json üzerinden — data/ büyük olabilir)
    let checksum_path = tmp_path.join("checksum.sha256");
    if checksum_path.exists() {
        let stored = fs::read_to_string(&checksum_path).unwrap_or_default();
        fs::remove_file(&checksum_path)?;
        let computed = compute_checksum(tmp_path)?;
        fs::write(&checksum_path, &stored)?;

        if stored.trim() != computed.trim() {
            return Err(RollbackError::InvalidInput(
                "Bütünlük doğrulaması başarısız: dosya bozulmuş olabilir".to_string(),
            ));
        }
    }

    let ts   = Local::now().format("%Y%m%d%H%M%S");
    let dest = target_base.join(format!("{}_{}_rx{}", meta.snapshot_id, meta.name, ts));

    match &meta.backend {
        BackendKind::Rsync => {
            // data/ doğrudan hedefe açılır — /tmp'ye kopyalanmaz
            import_rsync_direct(rxsnap_path, &dest)?;
        }
        BackendKind::Btrfs => {
            // data.btrfs.zst stdout'a çıkarılır, pipe ile btrfs receive'e gönderilir
            import_btrfs_direct(rxsnap_path, &dest)?;
        }
        BackendKind::LvmThin => {
            return Err(RollbackError::InvalidInput(
                "LVM Thin backend için import henüz desteklenmiyor".to_string(),
            ))
        }
    }

    Ok((meta, dest))
}

// ── Backend işlemleri ──────────────────────────────────────────────────────

fn resolve_rsync_path(backend_ref: &str) -> Result<PathBuf> {
    let src = if Path::new(backend_ref).is_absolute() {
        PathBuf::from(backend_ref)
    } else {
        PathBuf::from(RSYNC_SNAPSHOT_BASE).join(backend_ref)
    };
    if !src.exists() {
        return Err(RollbackError::InvalidInput(format!(
            "Snapshot dizini bulunamadı: {}",
            src.display()
        )));
    }
    Ok(src)
}

fn export_btrfs(backend_ref: &str, tmp: &Path) -> Result<()> {
    let stream = tmp.join("data.btrfs.zst");
    let subvol = if Path::new(backend_ref).is_absolute() {
        backend_ref.to_string()
    } else {
        format!("{BTRFS_SNAPSHOT_BASE}/{backend_ref}")
    };
    let btrfs = Command::new("btrfs")
        .args(["send", &subvol])
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let mut zstd = Command::new("zstd")
        .args(["-q", "-o", &stream.to_string_lossy()])
        .stdin(btrfs.stdout.unwrap())
        .spawn()?;
    let status = zstd.wait()?;
    if !status.success() {
        return Err(RollbackError::CommandFailed {
            cmd: "btrfs send | zstd".to_string(),
            reason: "Btrfs send başarısız".to_string(),
        });
    }
    Ok(())
}

/// Rsync import: arşivdeki data/ doğrudan hedefe açılır, /tmp'ye kopyalanmaz.
fn import_rsync_direct(archive: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    // --strip-components=1: "data/etc/passwd" → "etc/passwd"
    let status = Command::new("tar")
        .args([
            "--use-compress-program=zstd",
            "-xf",               &archive.to_string_lossy(),
            "-C",                &dest.to_string_lossy(),
            "--strip-components=1",
            "data",
        ])
        .status()?;
    if !status.success() {
        return Err(RollbackError::CommandFailed {
            cmd: "tar".to_string(),
            reason: "Snapshot verisi çıkarılamadı".to_string(),
        });
    }
    Ok(())
}

/// Btrfs import: data.btrfs.zst stdout'a çıkar → zstd -d → btrfs receive.
/// /tmp'ye yazılmaz.
fn import_btrfs_direct(archive: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(parent)?;

    // tar -xOf archive data.btrfs.zst | zstd -d | btrfs receive <parent>
    let mut tar_proc = Command::new("tar")
        .args([
            "--use-compress-program=zstd",
            "-xOf", &archive.to_string_lossy(),
            "./data.btrfs.zst",
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let mut zstd = Command::new("zstd")
        .arg("-d")
        .stdin(tar_proc.stdout.take().unwrap())
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let mut recv = Command::new("btrfs")
        .args(["receive", &parent.to_string_lossy()])
        .stdin(zstd.stdout.take().unwrap())
        .spawn()?;

    let status = recv.wait()?;
    if !status.success() {
        return Err(RollbackError::CommandFailed {
            cmd: "btrfs receive".to_string(),
            reason: "Btrfs receive başarısız".to_string(),
        });
    }
    Ok(())
}

// ── Arşiv işlemleri ───────────────────────────────────────────────────────

/// Btrfs/LVM: tmp dizinini olduğu gibi arşivle.
fn pack(src_dir: &Path, out: &Path) -> Result<()> {
    let status = Command::new("tar")
        .args([
            "--use-compress-program=zstd",
            "-cf", &out.to_string_lossy(),
            "-C",  &src_dir.to_string_lossy(),
            ".",
        ])
        .status()?;
    if !status.success() {
        return Err(RollbackError::CommandFailed {
            cmd: "tar".to_string(),
            reason: "Arşiv oluşturulamadı".to_string(),
        });
    }
    Ok(())
}

/// Rsync: snapshot_dir'i /tmp'ye kopyalamadan data/ olarak arşivle.
fn pack_rsync(snapshot_dir: &Path, meta_dir: &Path, out: &Path) -> Result<()> {
    let src_parent = snapshot_dir.parent().unwrap_or(Path::new("/"));
    let src_name   = snapshot_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let transform = format!("s|^{src_name}|data|");

    let status = Command::new("tar")
        .args([
            "--use-compress-program=zstd",
            "-cf",         &out.to_string_lossy(),
            "-C",          &meta_dir.to_string_lossy(),
            ".",           // metadata.json, checksum.sha256
            "-C",          &src_parent.to_string_lossy(),
            "--transform", &transform,
            &src_name,     // snapshot dizini → data/ olarak eklenir
        ])
        .status()?;

    if !status.success() {
        return Err(RollbackError::CommandFailed {
            cmd: "tar".to_string(),
            reason: "Arşiv oluşturulamadı".to_string(),
        });
    }
    Ok(())
}

/// Arşivden sadece belirtilen dosyaları çıkar (küçük metadata dosyaları için).
fn extract_named(archive: &Path, target: &Path, names: &[&str]) -> Result<()> {
    fs::create_dir_all(target)?;
    let mut cmd = Command::new("tar");
    cmd.arg("--use-compress-program=zstd")
       .arg("-xf").arg(archive)
       .arg("-C").arg(target);
    for n in names {
        cmd.arg(n);
    }
    // Bazı dosyalar arşivde olmayabilir; hata kodunu yoksay, sonraki adım kontrol eder
    let _ = cmd.status();
    Ok(())
}

// ── Checksum ──────────────────────────────────────────────────────────────

/// metadata.json + data.btrfs.zst üzerinden SHA256.
/// Rsync'in data/ dizini dahil edilmez (GB'larca olabilir).
fn compute_checksum(dir: &Path) -> Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for name in &["metadata.json", "data.btrfs.zst"] {
        let p = dir.join(name);
        if p.exists() { files.push(p); }
    }
    files.sort();

    if files.is_empty() {
        return Ok("empty".to_string());
    }

    let args: Vec<&str> = files.iter().map(|p| p.to_str().unwrap()).collect();
    let out = Command::new("sha256sum").args(&args[..]).output()?;

    let combined: String = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect::<Vec<_>>()
        .join("\n");

    let out2 = Command::new("sh")
        .args(["-c", &format!("printf '%s' '{}' | sha256sum | awk '{{print $1}}'", combined)])
        .output()?;
    Ok(String::from_utf8_lossy(&out2.stdout).trim().to_string())
}

// ── Yardımcılar ───────────────────────────────────────────────────────────

fn read_hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// `.rxsnap` arşivini açmadan metadata bilgilerini gösterir.
///
/// Arşivden sadece `metadata.json` çıkarılır ve parse edilir.
pub fn inspect(rxsnap_path: &Path) -> Result<RxSnapMeta> {
    if !rxsnap_path.exists() {
        return Err(RollbackError::InvalidInput(format!(
            "Dosya bulunamadı: {}",
            rxsnap_path.display()
        )));
    }
    let tmp = tempfile::TempDir::new()?;
    extract_named(rxsnap_path, tmp.path(), &["./metadata.json"])?;
    let meta_path = tmp.path().join("metadata.json");
    if !meta_path.exists() {
        return Err(RollbackError::InvalidInput(
            "Geçersiz .rxsnap: metadata.json bulunamadı".to_string(),
        ));
    }
    let meta: RxSnapMeta = serde_json::from_str(&fs::read_to_string(&meta_path)?)?;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rxsnap_meta_serialize() {
        let meta = RxSnapMeta {
            rx_version: "1.0.0".to_string(),
            format_version: FORMAT_VERSION,
            snapshot_id: 1,
            name: "test".to_string(),
            description: Some("açıklama".to_string()),
            created_at: "2026-01-01T00:00:00+03:00".to_string(),
            backend: BackendKind::Rsync,
            hostname: "pardus".to_string(),
            exported_at: "2026-01-01T12:00:00+03:00".to_string(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("Rsync"));

        let parsed: RxSnapMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.snapshot_id, 1);
        assert_eq!(parsed.format_version, FORMAT_VERSION);
    }

    #[test]
    fn test_export_nonexistent_snapshot() {
        let snap = Snapshot {
            id: 999,
            name: "ghost".to_string(),
            description: None,
            created_at: chrono::Local::now(),
            trigger: crate::snapshot::SnapshotTrigger::Manual,
            backend: BackendKind::Rsync,
            backend_ref: "nonexistent_dir".to_string(),
            size_bytes: None,
            locked: false,
        };
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("out.rxsnap");
        let result = export(&snap, &out);
        assert!(result.is_err());
    }

    #[test]
    fn test_import_nonexistent_file() {
        let result = import(
            std::path::Path::new("/tmp/nonexistent.rxsnap"),
            std::path::Path::new("/tmp"),
        );
        assert!(result.is_err());
    }
}
