use chrono::Local;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};


use crate::backend::{DogrulamaRaporu, RestoreRequirement, SnapshotBackend};
use crate::error::{Result, RollbackError};
use crate::snapshot::{BackendKind, Snapshot, SnapshotTrigger};

/// Rsync hard-link tabanlı snapshot backend.
/// Btrfs veya LVM Thin olmayan sistemlerde (ext4 vb.) çalışır.
/// Her snapshot önceki snapshot'a hard-link ile bağlanır → space-efficient.
pub struct RsyncBackend {
    /// Snapshot'ların saklandığı dizin
    snapshot_base: PathBuf,
    /// Rsync'in kaynak dizini (genellikle "/")
    source: PathBuf,
}

impl RsyncBackend {
    const DEFAULT_SNAPSHOT_BASE: &'static str = "/var/lib/rollbackx/snapshots";
    const DEFAULT_SOURCE: &'static str = "/";

    pub fn new() -> Self {
        RsyncBackend {
            snapshot_base: PathBuf::from(Self::DEFAULT_SNAPSHOT_BASE),
            source: PathBuf::from(Self::DEFAULT_SOURCE),
        }
    }

    pub fn with_paths(snapshot_base: &str, source: &str) -> Self {
        RsyncBackend {
            snapshot_base: PathBuf::from(snapshot_base),
            source: PathBuf::from(source),
        }
    }

    fn snapshot_dir_name(id: u32, name: &str) -> String {
        let ts = Local::now().format("%Y%m%d_%H%M%S");
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        format!("{id}_{safe}_{ts}")
    }

    fn snapshot_path(&self, dir_name: &str) -> PathBuf {
        self.snapshot_base.join(dir_name)
    }

    /// En son snapshot dizinini bul (hard-link referansı için)
    fn latest_snapshot_path(&self) -> Option<PathBuf> {
        let entries = fs::read_dir(&self.snapshot_base).ok()?;
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        dirs.into_iter().last()
    }

    /// Snapshot dizininin boyutunu hesapla (sadece exclusive baytlar)
    fn dir_size_exclusive(path: &Path) -> Option<u64> {
        // du --apparent-size ile toplam bayt al
        let output = Command::new("du")
            .args(["-sb", "--apparent-size"])
            .arg(path)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.split_whitespace().next()?.parse().ok()
    }

    /// Restore için pending marker dosyası yolu
    fn pending_restore_marker() -> PathBuf {
        PathBuf::from("/var/lib/rollbackx/pending-restore.conf")
    }

}

impl Default for RsyncBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotBackend for RsyncBackend {
    fn name(&self) -> &str {
        "Rsync"
    }

    fn is_available(&self) -> Result<bool> {
        // rsync kurulu mu?
        let ok = Command::new("rsync")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        Ok(ok)
    }

    fn create(
        &self,
        name: &str,
        desc: Option<&str>,
        trigger: SnapshotTrigger,
        next_id: u32,
    ) -> Result<Snapshot> {
        // Snapshot dizinini hazırla
        fs::create_dir_all(&self.snapshot_base)?;

        let dir_name = Self::snapshot_dir_name(next_id, name);
        let dest = self.snapshot_path(&dir_name);

        // Önceki snapshot hard-link referansı
        let mut cmd = Command::new("rsync");
        cmd.arg("-aHAXx")          // archive, hard-links, acl, xattr, exclude-other-fs
           .arg("--numeric-ids")
           .arg("--delete")
           .arg("--info=progress2")
           .arg("--exclude=/proc/")
           .arg("--exclude=/sys/")
           .arg("--exclude=/dev/")
           .arg("--exclude=/run/")
           .arg("--exclude=/tmp/")
           .arg("--exclude=/var/lib/rollbackx/snapshots/")
           .arg("--exclude=/lost+found");

        // Önceki snapshot varsa hard-link kullan (space-efficient)
        if let Some(prev) = self.latest_snapshot_path() {
            cmd.arg(format!("--link-dest={}", prev.display()));
        }

        cmd.arg(format!("{}/", self.source.display()))
           .arg(&dest)
           .stdout(Stdio::piped())
           .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| RollbackError::CommandFailed {
            cmd: "rsync".to_string(),
            reason: e.to_string(),
        })?;

        // stderr'i ayrı thread'de oku (deadlock önlemi)
        let stderr = child.stderr.take().unwrap();
        let stderr_thread = std::thread::spawn(move || {
            let mut s = String::new();
            BufReader::new(stderr).read_to_string(&mut s).ok();
            s
        });

        // stdout'u byte byte oku; rsync --info=progress2 \r ile günceller
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut buf = Vec::with_capacity(256);
        loop {
            buf.clear();
            let mut byte = [0u8; 1];
            loop {
                match reader.read(&mut byte) {
                    Ok(0) => break,               // EOF
                    Ok(_) => {
                        if byte[0] == b'\r' || byte[0] == b'\n' {
                            break;                // satır/progress sonu
                        }
                        buf.push(byte[0]);
                    }
                    Err(_) => break,
                }
            }
            if buf.is_empty() {
                // EOF kontrolü
                if reader.fill_buf().map_or(true, |b| b.is_empty()) {
                    break;
                }
                continue;
            }
            let line = String::from_utf8_lossy(&buf);
            // rsync --info=progress2 çıktısı: "  1,234,567  45%  12.34MB/s ..."
            let pct = line.split_whitespace()
                .find(|s| s.ends_with('%'))
                .and_then(|s| s.trim_end_matches('%').parse::<u8>().ok());
            if let Some(p) = pct {
                // GTK tarafının okuyabileceği özel format
                println!("PROGRESS:{p}");
                let _ = std::io::stdout().flush();
            }
        }

        let status = child.wait().map_err(|e| RollbackError::CommandFailed {
            cmd: "rsync".to_string(),
            reason: e.to_string(),
        })?;
        let stderr_content = stderr_thread.join().unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            if code != 24 {
                return Err(RollbackError::CommandFailed {
                    cmd: "rsync".to_string(),
                    reason: stderr_content.trim().to_string(),
                });
            }
        }

        let size_bytes = Self::dir_size_exclusive(&dest);

        Ok(Snapshot {
            id: next_id,
            name: name.to_string(),
            description: desc.map(|d| d.to_string()),
            created_at: Local::now(),
            trigger,
            backend: BackendKind::Rsync,
            backend_ref: dir_name,
            size_bytes,
            locked: false,
        })
    }

    fn restore(&self, snapshot: &Snapshot) -> Result<RestoreRequirement> {
        let snap_path = self.snapshot_path(&snapshot.backend_ref);

        if !snap_path.exists() {
            return Err(RollbackError::InvalidInput(format!(
                "Snapshot dizini bulunamadı: {}",
                snap_path.display()
            )));
        }

        // pending-restore.conf yaz — kalıcı kurulu servis bunu okur
        fs::write(Self::pending_restore_marker(), snap_path.display().to_string())?;

        Ok(RestoreRequirement::Reboot)
    }

    fn delete(&self, snapshot: &Snapshot) -> Result<()> {
        let snap_path = self.snapshot_path(&snapshot.backend_ref);
        if snap_path.exists() {
            fs::remove_dir_all(&snap_path)?;
        }
        Ok(())
    }

    fn refresh_size(&self, snapshot: &Snapshot) -> Option<u64> {
        let snap_path = self.snapshot_path(&snapshot.backend_ref);
        Self::dir_size_exclusive(&snap_path)
    }

    fn mount(&self, snapshot: &Snapshot, mount_point: &Path) -> Result<()> {
        let snap_path = self.snapshot_path(&snapshot.backend_ref);
        let mp = mount_point.to_string_lossy().to_string();
        fs::create_dir_all(mount_point)?;

        // ro,bind tek seferde — remount,ro sonraki adım olarak "busy" verir
        let out = Command::new("mount")
            .args(["-o", "ro,bind", &snap_path.to_string_lossy(), &mp])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: "mount -o ro,bind".to_string(),
                reason: e.to_string(),
            })?;
        if !out.status.success() {
            return Err(RollbackError::CommandFailed {
                cmd: "mount -o ro,bind".to_string(),
                reason: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(())
    }

    fn verify(&self, snapshot: &Snapshot) -> Result<DogrulamaRaporu> {
        let snap_path = self.snapshot_path(&snapshot.backend_ref);
        if !snap_path.exists() {
            return Err(RollbackError::InvalidInput(format!(
                "Snapshot dizini bulunamadı: {}",
                snap_path.display()
            )));
        }
        Ok(crate::backend::verify_snapshot_dir(&snap_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_dir_name_format() {
        let name = RsyncBackend::snapshot_dir_name(1, "test snapshot");
        assert!(name.starts_with("1_test-snapshot_"));
        assert!(name.len() > 16); // id + name + timestamp
    }
}
