use clap::Subcommand;
use std::process::Command;

use rollbackx_core::{
    error::{Result, RollbackError},
    snapshot::{BackendKind, SnapshotDB},
};

use crate::output;

const AUTO_CONF: &str = "/var/lib/rollbackx/auto-restore.conf";
const SERVICE:   &str = "rollbackx-auto-restore.service";

#[derive(Subcommand)]
pub enum OtoKomut {
    /// Her açılışta geri yüklenecek snapshot'ı ayarla
    #[command(name = "kur")]
    Kur {
        /// Hedef snapshot ID'si
        id: u32,
    },
    /// Önyükleme otomatik geri yüklemeyi kaldır
    #[command(name = "kaldir")]
    Kaldir,
    /// Mevcut otomatik geri yükleme hedefini göster
    #[command(name = "goster")]
    Goster,
}

pub fn calistir(komut: OtoKomut) -> Result<()> {
    match komut {
        OtoKomut::Kur { id } => kur(id),
        OtoKomut::Kaldir     => kaldir(),
        OtoKomut::Goster     => goster(),
    }
}

fn root_kontrol() -> Result<()> {
    extern "C" { fn getuid() -> u32; }
    if unsafe { getuid() } != 0 {
        return Err(RollbackError::PermissionDenied);
    }
    Ok(())
}

fn kur(id: u32) -> Result<()> {
    root_kontrol()?;

    let db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;

    if snap.backend != BackendKind::Rsync {
        return Err(RollbackError::InvalidInput(
            "Önyükleme otomatik geri yükleme şu an yalnızca Rsync backend ile çalışır.".to_string(),
        ));
    }

    let snap_path = format!("/var/lib/rollbackx/snapshots/{}", snap.backend_ref);
    if !std::path::Path::new(&snap_path).exists() {
        return Err(RollbackError::InvalidInput(format!(
            "Snapshot dizini bulunamadı: {snap_path}"
        )));
    }

    std::fs::create_dir_all("/var/lib/rollbackx")?;
    std::fs::write(AUTO_CONF, &snap_path)?;

    // Servisi etkinleştir (hata olsa bile devam et — systemd olmayan ortamlar)
    let _ = Command::new("systemctl")
        .args(["enable", SERVICE])
        .output();

    output::basari(&format!(
        "Önyükleme otomatik geri yükleme ayarlandı: Snapshot #{id} \"{}\"",
        snap.name
    ));
    output::bilgi("Her açılışta bu snapshot'a otomatik geri yüklenecek.");
    output::bilgi(&format!("Hedef: {snap_path}"));
    Ok(())
}

fn kaldir() -> Result<()> {
    root_kontrol()?;

    if std::path::Path::new(AUTO_CONF).exists() {
        std::fs::remove_file(AUTO_CONF)?;
        output::basari("Önyükleme otomatik geri yükleme devre dışı bırakıldı.");
    } else {
        output::bilgi("Önyükleme otomatik geri yükleme zaten aktif değil.");
    }
    Ok(())
}

fn goster() -> Result<()> {
    if !std::path::Path::new(AUTO_CONF).exists() {
        output::bilgi("Önyükleme otomatik geri yükleme: Devre dışı");
        return Ok(());
    }

    let snap_path = std::fs::read_to_string(AUTO_CONF)
        .unwrap_or_default()
        .trim()
        .to_string();

    let isim = SnapshotDB::open(None)
        .ok()
        .and_then(|db| {
            db.all()
                .iter()
                .find(|s| snap_path.contains(&s.backend_ref))
                .map(|s| format!("#{} \"{}\"", s.id, s.name))
        })
        .unwrap_or_else(|| snap_path.clone());

    output::basari(&format!("Önyükleme otomatik geri yükleme: AKTİF → {isim}"));
    output::bilgi(&format!("Hedef dizin: {snap_path}"));
    Ok(())
}
