use chrono::Local;
use std::fs;
use std::path::Path;
use std::process::Command;


use crate::backend::{DogrulamaRaporu, RestoreRequirement, SnapshotBackend};
use crate::error::{Result, RollbackError};
use crate::snapshot::{BackendKind, Snapshot, SnapshotTrigger};

/// Btrfs subvolume tabanlı snapshot backend'i.
///
/// Salt-okunur snapshot'lar oluşturur (`btrfs subvolume snapshot -r`).
/// Geri yükleme `btrfs subvolume set-default` ile yapılır ve reboot gerektirir.
pub struct BtrfsBackend {
    /// Kök subvolume'ün bağlı olduğu nokta (genellikle "/")
    root_mount: String,
    /// Snapshot dizini (genellikle "/.snapshots")
    snapshot_dir: String,
}

impl BtrfsBackend {
    /// Varsayılan ayarlarla yeni bir Btrfs backend oluşturur.
    ///
    /// Root mount: `/`, snapshot dizini: `/.snapshots`
    pub fn new() -> Self {
        BtrfsBackend {
            root_mount: "/".to_string(),
            snapshot_dir: "/.snapshots".to_string(),
        }
    }

    /// Test ortamı için özel mount noktası ile oluştur
    pub fn with_paths(root_mount: &str, snapshot_dir: &str) -> Self {
        BtrfsBackend {
            root_mount: root_mount.to_string(),
            snapshot_dir: snapshot_dir.to_string(),
        }
    }

    /// `/proc/self/mountinfo` parse ederek Btrfs kök subvolume'ü bul
    fn find_btrfs_root() -> Result<Option<String>> {
        let content = fs::read_to_string("/proc/self/mountinfo")
            .map_err(RollbackError::Io)?;

        for line in content.lines() {
            // Format: id parent major:minor root mount_point mount_opts ... fs_type ...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 9 {
                continue;
            }
            let mount_point = parts[4];
            // Separator '-' index bul
            let sep_idx = parts.iter().position(|&p| p == "-");
            if let Some(idx) = sep_idx {
                if parts.len() > idx + 1 && parts[idx + 1] == "btrfs" && mount_point == "/" {
                    // subvol=@ var mı diye root alanını kontrol et (parts[3])
                    return Ok(Some(mount_point.to_string()));
                }
            }
        }
        Ok(None)
    }

    fn snapshot_subvol_name(id: u32, name: &str) -> String {
        let ts = Local::now().format("%Y%m%d_%H%M%S");
        // Adı URL-safe yap: boşluk → '-'
        let safe_name: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        format!("@{id}_{safe_name}_{ts}")
    }

    fn run_btrfs(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("btrfs")
            .args(args)
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: format!("btrfs {}", args.join(" ")),
                reason: e.to_string(),
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(RollbackError::CommandFailed {
                cmd: format!("btrfs {}", args.join(" ")),
                reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }

    /// Subvolume ID'sini `btrfs subvolume show` ile al
    fn get_subvol_id(&self, path: &str) -> Option<u64> {
        let output = Command::new("btrfs")
            .args(["subvolume", "show", path])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.trim_start().starts_with("Subvolume ID:") {
                return line.split(':').nth(1)?.trim().parse().ok();
            }
        }
        None
    }

    /// Disk kullanımını `btrfs filesystem du` ile tahmin et
    fn get_size(&self, path: &str) -> Option<u64> {
        let output = Command::new("btrfs")
            .args(["filesystem", "du", "--summarize", path])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        // İlk satır başlık, ikinci satır veri: "  SIZE  EXCLUSIVE  SET SHARED  FILENAME"
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if !parts.is_empty() {
                // "1.50GiB" gibi string parse et
                return parse_btrfs_size(parts[0]);
            }
        }
        None
    }
}

fn parse_btrfs_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s == "-" || s.is_empty() {
        return None;
    }
    let (num, unit) = if let Some(idx) = s.find(|c: char| c.is_alphabetic()) {
        (&s[..idx], &s[idx..])
    } else {
        return s.parse().ok();
    };
    let val: f64 = num.parse().ok()?;
    let mult = match unit.to_lowercase().as_str() {
        "b"                     => 1u64,
        "k" | "kib" | "kb"     => 1024,
        "m" | "mib" | "mb"     => 1024 * 1024,
        "g" | "gib" | "gb"     => 1024 * 1024 * 1024,
        "t" | "tib" | "tb"     => 1024 * 1024 * 1024 * 1024,
        _                       => return None,
    };
    Some((val * mult as f64) as u64)
}

impl Default for BtrfsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotBackend for BtrfsBackend {
    fn name(&self) -> &str {
        "Btrfs"
    }

    fn is_available(&self) -> Result<bool> {
        // 1. /proc/self/mountinfo'da btrfs kök mount var mı?
        if Self::find_btrfs_root()?.is_none() {
            return Ok(false);
        }
        // 2. btrfs komutu var mı?
        let status = Command::new("btrfs").arg("--version").output();
        Ok(status.map(|o| o.status.success()).unwrap_or(false))
    }

    fn create(
        &self,
        name: &str,
        desc: Option<&str>,
        trigger: SnapshotTrigger,
        next_id: u32,
    ) -> Result<Snapshot> {
        let subvol_name = Self::snapshot_subvol_name(next_id, name);
        let dest = format!("{}/{}", self.snapshot_dir, subvol_name);

        // Dizin yoksa oluştur
        if !Path::new(&self.snapshot_dir).exists() {
            fs::create_dir_all(&self.snapshot_dir)?;
        }

        self.run_btrfs(&["subvolume", "snapshot", "-r", &self.root_mount, &dest])?;

        let size_bytes = self.get_size(&dest);

        Ok(Snapshot {
            id: next_id,
            name: name.to_string(),
            description: desc.map(|d| d.to_string()),
            created_at: Local::now(),
            trigger,
            backend: BackendKind::Btrfs,
            backend_ref: subvol_name,
            size_bytes,
            locked: false,
        })
    }

    fn restore(&self, snapshot: &Snapshot) -> Result<RestoreRequirement> {
        let snap_path = format!("{}/{}", self.snapshot_dir, snapshot.backend_ref);

        // Subvolume ID'sini al
        let subvol_id = self
            .get_subvol_id(&snap_path)
            .ok_or_else(|| RollbackError::CommandFailed {
                cmd: "btrfs subvolume show".to_string(),
                reason: format!("Subvolume ID alınamadı: {snap_path}"),
            })?;

        // set-default ile varsayılan subvolume'ü değiştir
        self.run_btrfs(&[
            "subvolume",
            "set-default",
            &subvol_id.to_string(),
            &self.root_mount,
        ])?;

        Ok(RestoreRequirement::Reboot)
    }

    fn delete(&self, snapshot: &Snapshot) -> Result<()> {
        let snap_path = format!("{}/{}", self.snapshot_dir, snapshot.backend_ref);
        self.run_btrfs(&["subvolume", "delete", &snap_path])?;
        Ok(())
    }

    fn refresh_size(&self, snapshot: &Snapshot) -> Option<u64> {
        let snap_path = format!("{}/{}", self.snapshot_dir, snapshot.backend_ref);
        self.get_size(&snap_path)
    }

    fn mount(&self, snapshot: &Snapshot, mount_point: &Path) -> Result<()> {
        let snap_path = format!("{}/{}", self.snapshot_dir, snapshot.backend_ref);
        let mp = mount_point.to_string_lossy().to_string();
        fs::create_dir_all(mount_point)?;

        // Btrfs snapshot'ları zaten read-only oluşturuluyor (-r flag),
        // ro,bind tek seferde yeterli
        let out = Command::new("mount")
            .args(["-o", "ro,bind", &snap_path, &mp])
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
        let snap_path = std::path::PathBuf::from(&self.snapshot_dir).join(&snapshot.backend_ref);
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
    fn test_snapshot_name_format() {
        let name = BtrfsBackend::snapshot_subvol_name(1, "test snapshot");
        assert!(name.starts_with("@1_test-snapshot_"));
    }

    #[test]
    fn test_parse_btrfs_size() {
        assert_eq!(parse_btrfs_size("1024"), Some(1024));
        assert_eq!(parse_btrfs_size("1KiB"), Some(1024));
        assert_eq!(parse_btrfs_size("1.5GiB"), Some(1_610_612_736));
        assert_eq!(parse_btrfs_size("-"), None);
    }
}
