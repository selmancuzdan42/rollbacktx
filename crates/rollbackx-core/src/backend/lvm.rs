use chrono::Local;
use std::process::Command;

use std::path::Path;

use crate::backend::{DogrulamaRaporu, RestoreRequirement, SnapshotBackend};
use crate::error::{Result, RollbackError};
use crate::snapshot::{BackendKind, Snapshot, SnapshotTrigger};

/// LVM thin provisioning tabanlı snapshot backend'i.
///
/// `lvcreate --snapshot --thin` ile snapshot oluşturur.
/// Geri yükleme `lvconvert --merge` ile yapılır ve reboot gerektirir.
pub struct LvmThinBackend {
    /// Volume group adı (örn: "debian-vg")
    vg_name: String,
    /// Root LV adı (örn: "root")
    root_lv: String,
}

impl LvmThinBackend {
    /// Varsayılan ayarlarla yeni bir LVM Thin backend oluşturur.
    ///
    /// VG/LV adları `is_available()` çağrısında otomatik keşfedilir.
    pub fn new() -> Self {
        LvmThinBackend {
            vg_name: String::new(),
            root_lv: String::new(),
        }
    }

    /// Belirtilen VG ve LV adlarıyla yeni bir LVM Thin backend oluşturur.
    pub fn with_vg_lv(vg_name: &str, root_lv: &str) -> Self {
        LvmThinBackend {
            vg_name: vg_name.to_string(),
            root_lv: root_lv.to_string(),
        }
    }

    /// `lvs` çıktısını parse ederek root LV'nin thin pool'da olduğunu doğrula.
    /// (vg_name, lv_name) döndürür.
    fn detect_thin_root() -> Result<Option<(String, String)>> {
        let output = Command::new("lvs")
            .args([
                "--noheadings",
                "--separator=|",
                "-o",
                "vg_name,lv_name,lv_attr,pool_lv",
            ])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: "lvs".to_string(),
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split('|').map(str::trim).collect();
            if parts.len() < 4 {
                continue;
            }
            let (vg, lv, attr, pool) = (parts[0], parts[1], parts[2], parts[3]);
            // lv_attr[0] == 'V' → thin volume
            // lv == "root" (veya "/" mount'u olan LV)
            if attr.starts_with('V') && !pool.is_empty() && lv == "root" {
                return Ok(Some((vg.to_string(), lv.to_string())));
            }
        }
        Ok(None)
    }

    fn lv_path(&self, lv_name: &str) -> String {
        format!("/dev/{}/{}", self.vg_name, lv_name)
    }

    fn snapshot_lv_name(id: u32, name: &str) -> String {
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        format!("rx_{id}_{safe}")
    }

    fn run_cmd(&self, program: &str, args: &[&str]) -> Result<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: format!("{program} {}", args.join(" ")),
                reason: e.to_string(),
            })?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(RollbackError::CommandFailed {
                cmd: format!("{program} {}", args.join(" ")),
                reason: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }

    /// LV boyutunu bytes cinsinden al
    fn get_lv_size(&self, lv_name: &str) -> Option<u64> {
        let output = Command::new("lvs")
            .args([
                "--noheadings",
                "--nosuffix",
                "--units=b",
                "-o",
                "lv_size",
                &self.lv_path(lv_name),
            ])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.trim().parse().ok()
    }
}

impl Default for LvmThinBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotBackend for LvmThinBackend {
    fn name(&self) -> &str {
        "LVM Thin"
    }

    fn is_available(&self) -> Result<bool> {
        // lvs komutu var mı?
        let lvs_ok = Command::new("lvs")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !lvs_ok {
            return Ok(false);
        }
        // Root LV thin pool'da mı?
        Ok(Self::detect_thin_root()?.is_some())
    }

    fn create(
        &self,
        name: &str,
        desc: Option<&str>,
        trigger: SnapshotTrigger,
        next_id: u32,
    ) -> Result<Snapshot> {
        // is_available() ile VG/LV adları dolmamışsa keşfet
        let (vg, lv_root) = if self.vg_name.is_empty() {
            Self::detect_thin_root()?.ok_or(RollbackError::BackendNotAvailable(
                "LVM Thin root LV bulunamadı".to_string(),
            ))?
        } else {
            (self.vg_name.clone(), self.root_lv.clone())
        };

        let snap_lv = Self::snapshot_lv_name(next_id, name);
        let root_path = format!("/dev/{vg}/{lv_root}");

        self.run_cmd(
            "lvcreate",
            &["--snapshot", "--thin", "-n", &snap_lv, &root_path],
        )?;

        let size_bytes = self.get_lv_size(&snap_lv);

        Ok(Snapshot {
            id: next_id,
            name: name.to_string(),
            description: desc.map(|d| d.to_string()),
            created_at: Local::now(),
            trigger,
            backend: BackendKind::LvmThin,
            backend_ref: format!("{vg}/{snap_lv}"),
            size_bytes,
            locked: false,
        })
    }

    fn restore(&self, snapshot: &Snapshot) -> Result<RestoreRequirement> {
        // backend_ref = "vg/lv_name"
        let lv_path = format!("/dev/{}", snapshot.backend_ref);
        self.run_cmd("lvconvert", &["--merge", &lv_path])?;
        Ok(RestoreRequirement::Reboot)
    }

    fn delete(&self, snapshot: &Snapshot) -> Result<()> {
        let lv_path = format!("/dev/{}", snapshot.backend_ref);
        self.run_cmd("lvremove", &["-f", &lv_path])?;
        Ok(())
    }

    fn refresh_size(&self, snapshot: &Snapshot) -> Option<u64> {
        // backend_ref = "vg/lv_name" → lv_name son parça
        let lv_name = snapshot.backend_ref.split('/').next_back()?;
        self.get_lv_size(lv_name)
    }

    fn mount(&self, snapshot: &Snapshot, mount_point: &Path) -> Result<()> {
        // backend_ref = "vg/lv"
        let lv_path = format!("/dev/{}", snapshot.backend_ref);
        let mp = mount_point.to_string_lossy().to_string();
        std::fs::create_dir_all(mount_point)?;

        // LV'yi aktif et
        let out = Command::new("lvchange")
            .args(["-ay", &lv_path])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: format!("lvchange -ay {lv_path}"),
                reason: e.to_string(),
            })?;
        if !out.status.success() {
            return Err(RollbackError::CommandFailed {
                cmd: format!("lvchange -ay {lv_path}"),
                reason: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }

        // Read-only mount
        let out2 = Command::new("mount")
            .args(["-o", "ro", &lv_path, &mp])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: format!("mount -o ro {lv_path} {mp}"),
                reason: e.to_string(),
            })?;
        if !out2.status.success() {
            // Başarısız olursa LV'yi deaktif et
            let _ = Command::new("lvchange").args(["-an", &lv_path]).output();
            return Err(RollbackError::CommandFailed {
                cmd: format!("mount -o ro {lv_path} {mp}"),
                reason: String::from_utf8_lossy(&out2.stderr).trim().to_string(),
            });
        }
        Ok(())
    }

    fn verify(&self, snapshot: &Snapshot) -> Result<DogrulamaRaporu> {
        let lv_path = format!("/dev/{}", snapshot.backend_ref);
        let mp = std::path::PathBuf::from(format!("/tmp/rollbackx-verify-{}", snapshot.id));

        // LV'yi aktif et
        let out = Command::new("lvchange")
            .args(["-ay", &lv_path])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: format!("lvchange -ay {lv_path}"),
                reason: e.to_string(),
            })?;
        if !out.status.success() {
            return Err(RollbackError::CommandFailed {
                cmd: format!("lvchange -ay {lv_path}"),
                reason: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }

        std::fs::create_dir_all(&mp)?;

        let out2 = Command::new("mount")
            .args(["-o", "ro", &lv_path, mp.to_str().unwrap_or("")])
            .output()
            .map_err(|e| RollbackError::CommandFailed {
                cmd: "mount".to_string(),
                reason: e.to_string(),
            })?;
        if !out2.status.success() {
            let _ = std::fs::remove_dir(&mp);
            let _ = Command::new("lvchange").args(["-an", &lv_path]).output();
            return Err(RollbackError::CommandFailed {
                cmd: "mount".to_string(),
                reason: String::from_utf8_lossy(&out2.stderr).trim().to_string(),
            });
        }

        let rapor = crate::backend::verify_snapshot_dir(&mp);

        let _ = Command::new("umount").arg(&mp).output();
        let _ = std::fs::remove_dir(&mp);
        let _ = Command::new("lvchange").args(["-an", &lv_path]).output();

        Ok(rapor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_lv_name() {
        let lv = LvmThinBackend::snapshot_lv_name(3, "test snap");
        assert_eq!(lv, "rx_3_test-snap");
    }
}
