//! `rollbackx zamanlayici` — zamanlanmış otomatik geri yükleme yönetimi.
//!
//! Alt komutlar:
//!   kur  <snapshot-id> --siklik <gunluk|haftalik|aylik>
//!        [--gun <pazartesi|...>] [--kacinci <1-4>] [--saat <HH:MM>]
//!   goster        → aktif zamanlayıcıyı göster
//!   kaldir        → zamanlayıcıyı devre dışı bırak
//!   kontrol       → systemd, bugün tetiklenecek mi?

use clap::Subcommand;
use rollbackx_core::{
    error::Result,
    schedule::{Schedule, ScheduleGun, Siklik},
    snapshot::SnapshotDB,
};

use crate::output::{baslik, basari, bilgi, hata_yaz};

// ── Alt komut tanımı ──────────────────────────────────────────────────────────

#[derive(Subcommand)]
pub enum ZamanKomut {
    /// Zamanlayıcıyı kur
    #[command(name = "kur")]
    Kur {
        /// Geri yüklenecek snapshot ID'si
        snapshot_id: u32,

        /// Sıklık: gunluk | haftalik | aylik
        #[arg(long, value_name = "SIKLIK")]
        siklik: String,

        /// Haftanın günü (haftalık/aylık için): pazartesi, sali, carsamba...
        #[arg(long, value_name = "GUN")]
        gun: Option<String>,

        /// Ayın kaçıncı haftası (aylık için, 1-4)
        #[arg(long, value_name = "N")]
        kacinci: Option<u8>,

        /// Saat HH:MM formatında (varsayılan: 03:00)
        #[arg(long, value_name = "SAAT", default_value = "03:00")]
        saat: String,
    },

    /// Aktif zamanlayıcıyı göster
    #[command(name = "goster")]
    Goster,

    /// Zamanlayıcıyı kaldır
    #[command(name = "kaldir")]
    Kaldir,

    /// Zamanlayıcı bugün tetiklenmeli mi? (systemd tarafından çağrılır)
    #[command(name = "kontrol", hide = true)]
    Kontrol,
}

// ── Ana çalıştırıcı ───────────────────────────────────────────────────────────

pub fn calistir(komut: ZamanKomut) -> Result<()> {
    match komut {
        ZamanKomut::Kur { snapshot_id, siklik, gun, kacinci, saat } => {
            kur(snapshot_id, &siklik, gun.as_deref(), kacinci, &saat)
        }
        ZamanKomut::Goster => goster(),
        ZamanKomut::Kaldir => kaldir(),
        ZamanKomut::Kontrol => kontrol(),
    }
}

// ── kur ───────────────────────────────────────────────────────────────────────

fn kur(
    snapshot_id: u32,
    siklik_str:  &str,
    gun_str:     Option<&str>,
    kacinci:     Option<u8>,
    saat:        &str,
) -> Result<()> {
    // Snapshot var mı?
    let db = SnapshotDB::open(None)?;
    if db.get(snapshot_id).is_err() {
        hata_yaz(&format!("Snapshot #{snapshot_id} bulunamadı."));
        std::process::exit(1);
    }

    let siklik: Siklik = siklik_str.parse()?;

    let gun: Option<ScheduleGun> = match gun_str {
        Some(s) => Some(s.parse()?),
        None    => None,
    };

    // Doğrulama
    if matches!(siklik, Siklik::Haftalik | Siklik::Aylik) && gun.is_none() {
        hata_yaz("Haftalık/aylık sıklık için --gun belirtmelisiniz.");
        std::process::exit(1);
    }
    if let Some(n) = kacinci {
        if !(1..=4).contains(&n) {
            hata_yaz("--kacinci değeri 1 ile 4 arasında olmalıdır.");
            std::process::exit(1);
        }
    }
    validate_saat(saat)?;

    let schedule = Schedule {
        enabled: true,
        snapshot_id,
        siklik,
        gun,
        kacinci,
        saat: saat.to_string(),
    };

    schedule.kaydet()?;

    // Systemd timer'ı kullanıcının seçtiği saate güncelle
    timer_saatini_guncelle(saat);

    basari(&format!("Zamanlayıcı kuruldu: {}", schedule.acikla()));
    bilgi("Systemd timer aktif: rollbackx-schedule.timer");
    Ok(())
}

/// Systemd timer'ın OnCalendar saatini günceller.
/// `/etc/systemd/system/rollbackx-schedule.timer.d/override.conf` dosyasına yazar.
fn timer_saatini_guncelle(saat: &str) {
    let override_dir = "/etc/systemd/system/rollbackx-schedule.timer.d";
    let override_path = format!("{override_dir}/override.conf");

    if std::fs::create_dir_all(override_dir).is_err() {
        return;
    }

    let icerik = format!(
        "[Timer]\nOnCalendar=\nOnCalendar=*-*-* {saat}:00\n"
    );
    if std::fs::write(&override_path, icerik).is_err() {
        return;
    }

    // daemon-reload + timer restart
    let _ = std::process::Command::new("systemctl")
        .args(["daemon-reload"])
        .status();
    let _ = std::process::Command::new("systemctl")
        .args(["restart", "rollbackx-schedule.timer"])
        .status();
}

// ── goster ────────────────────────────────────────────────────────────────────

fn goster() -> Result<()> {
    baslik("Zamanlayıcı Durumu");
    match Schedule::yukle()? {
        None => {
            bilgi("Aktif zamanlayıcı yok.");
        }
        Some(s) => {
            let durum = if s.enabled { "Aktif" } else { "Devre dışı" };
            println!("  Durum       : {durum}");
            println!("  Snapshot    : #{}", s.snapshot_id);
            println!("  Zamanlama   : {}", s.acikla());
            println!("  Bugün geçerli: {}", if s.tetikle_mi() { "Evet" } else { "Hayır" });
        }
    }
    Ok(())
}

// ── kaldir ────────────────────────────────────────────────────────────────────

fn kaldir() -> Result<()> {
    Schedule::kaldir()?;
    // Timer override'ı temizle
    let _ = std::fs::remove_file("/etc/systemd/system/rollbackx-schedule.timer.d/override.conf");
    let _ = std::fs::remove_dir("/etc/systemd/system/rollbackx-schedule.timer.d");
    let _ = std::process::Command::new("systemctl").args(["daemon-reload"]).status();
    basari("Zamanlayıcı kaldırıldı.");
    Ok(())
}

// ── kontrol (systemd tarafından çağrılır) ─────────────────────────────────────

fn kontrol() -> Result<()> {
    let schedule = match Schedule::yukle()? {
        None => {
            // Zamanlayıcı yoksa sessizce çık
            return Ok(());
        }
        Some(s) => s,
    };

    if !schedule.tetikle_mi() {
        // Bugün değil, sessizce çık
        return Ok(());
    }

    let snapshot_id = schedule.snapshot_id;

    // Snapshot var mı kontrol et
    let db = SnapshotDB::open(None)?;
    if db.get(snapshot_id).is_err() {
        hata_yaz(&format!(
            "Zamanlayıcı: Snapshot #{snapshot_id} bulunamadı. \
             Lütfen `rollbackx zamanlayici kaldir` ile zamanlayıcıyı güncelleyin."
        ));
        return Ok(());
    }

    println!("Zamanlayıcı tetiklendi: Snapshot #{snapshot_id} için geri yükleme başlatılıyor...");

    // Mevcut restore akışını kullan — backend pending-restore.conf'u doğru formatta yazar
    let status = std::process::Command::new("rollbackx")
        .args(["snapshot", "restore", &snapshot_id.to_string(), "--zorla"])
        .status()
        .map_err(|e| rollbackx_core::error::RollbackError::CommandFailed {
            cmd: "rollbackx".to_string(),
            reason: e.to_string(),
        })?;

    if status.success() {
        println!("Zamanlayıcı: Geri yükleme hazırlandı. Sistem yeniden başlatılıyor...");
        // systemd-logind üzerinden güvenli yeniden başlatma
        let _ = std::process::Command::new("systemctl")
            .arg("reboot")
            .status();
    } else {
        hata_yaz("Zamanlayıcı: Geri yükleme komutu başarısız oldu.");
    }

    Ok(())
}

// ── Yardımcılar ───────────────────────────────────────────────────────────────

fn validate_saat(saat: &str) -> Result<()> {
    let parts: Vec<&str> = saat.split(':').collect();
    let gecerli = parts.len() == 2
        && parts[0].parse::<u8>().is_ok_and(|h| h < 24)
        && parts[1].parse::<u8>().is_ok_and(|m| m < 60);
    if !gecerli {
        hata_yaz(&format!("Geçersiz saat formatı: '{saat}'. HH:MM bekleniyor (örn. 03:00)"));
        std::process::exit(1);
    }
    Ok(())
}
