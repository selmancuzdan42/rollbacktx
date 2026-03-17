use clap::Subcommand;

use rollbackx_core::{
    backend::{cleanup_stale_mount_points, detect_backend, is_snapshot_mounted, snapshot_mount_point, unmount_snapshot, RestoreRequirement},
    error::{Result, RollbackError},
    snapshot::{SnapshotDB, SnapshotTrigger},
};

use crate::output;

#[derive(Subcommand)]
pub enum SnapKomut {
    /// Yeni bir snapshot oluştur
    #[command(name = "create", alias = "oluştur")]
    Olustur {
        /// Snapshot ismi
        isim: String,
        /// Açıklama (opsiyonel)
        #[arg(long = "açıklama", short = 'a')]
        aciklama: Option<String>,
        /// Sessiz mod — başarı/bilgi mesajlarını bastır
        #[arg(long = "sessiz", short = 's')]
        sessiz: bool,
    },
    /// Snapshot listesini göster
    #[command(name = "list", alias = "listele")]
    Listele {
        /// JSON formatında çıktı
        #[arg(long = "json")]
        json: bool,
    },
    /// Bir snapshot'ı geri yükle
    #[command(name = "restore", alias = "geri-yükle")]
    GeriYukle {
        /// Geri yüklenecek snapshot ID'si
        id: u32,
        /// Onay sormadan zorla geri yükle
        #[arg(long = "zorla", short = 'z')]
        zorla: bool,
        /// Reboot gerekse bile reboot yapma (uyarı ver)
        #[arg(long = "yeniden-başlatma-yok")]
        reboot_yok: bool,
    },
    /// Bir snapshot'ı sil
    #[command(name = "delete", alias = "sil")]
    Sil {
        /// Silinecek snapshot ID'si
        id: u32,
        /// Onay sormadan zorla sil
        #[arg(long = "zorla", short = 'z')]
        zorla: bool,
    },
    /// Snapshot detaylarını göster
    #[command(name = "info", alias = "bilgi")]
    Bilgi {
        /// Bilgi gösterilecek snapshot ID'si
        id: u32,
    },
    /// Snapshot'ı /mnt/rollbackx/{id}/ altına read-only bağla
    #[command(name = "bagla")]
    Bagla {
        /// Bağlanacak snapshot ID'si
        id: u32,
    },
    /// Bağlı snapshot'ı ayır (umount)
    #[command(name = "ayir")]
    Ayir {
        /// Ayrılacak snapshot ID'si
        id: u32,
    },
    /// Snapshot'ı kilitle (silinemez, geri yüklenemez)
    #[command(name = "kilitle")]
    Kilitle {
        /// Kilitlenecek snapshot ID'si
        id: u32,
    },
    /// Snapshot kilidini kaldır
    #[command(name = "kilit-ac")]
    KilitAc {
        /// Kilidi açılacak snapshot ID'si
        id: u32,
    },
    /// Snapshot içeriğini doğrula (sistem dosyalarını kontrol et)
    #[command(name = "dogrula")]
    Dogrula {
        /// Doğrulanacak snapshot ID'si
        id: u32,
    },
    /// Snapshot adını ve açıklamasını değiştir
    #[command(name = "yeniden-adlandir")]
    YenidenAdlandir {
        /// Düzenlenecek snapshot ID'si
        id: u32,
        /// Yeni isim
        isim: String,
        /// Yeni açıklama (opsiyonel; belirtilmezse mevcut silinir)
        #[arg(long = "aciklama", short = 'a')]
        aciklama: Option<String>,
    },
}

pub fn calistir(komut: SnapKomut) -> Result<()> {
    match komut {
        SnapKomut::Olustur { isim, aciklama, sessiz } => olustur(&isim, aciklama.as_deref(), sessiz),
        SnapKomut::Listele { json }                   => listele(json),
        SnapKomut::GeriYukle { id, zorla, reboot_yok } => geri_yukle(id, zorla, reboot_yok),
        SnapKomut::Sil { id, zorla }                  => sil(id, zorla),
        SnapKomut::Bilgi { id }                       => bilgi(id),
        SnapKomut::Bagla { id }                       => bagla(id),
        SnapKomut::Ayir { id }                        => ayir(id),
        SnapKomut::Kilitle { id }                     => kilitle(id),
        SnapKomut::KilitAc { id }                     => kilit_ac(id),
        SnapKomut::Dogrula { id }                     => dogrula(id),
        SnapKomut::YenidenAdlandir { id, isim, aciklama } => yeniden_adlandir(id, isim, aciklama),
    }
}

fn root_kontrol() -> Result<()> {
    extern "C" {
        fn getuid() -> u32;
    }
    if unsafe { getuid() } != 0 {
        return Err(RollbackError::PermissionDenied);
    }
    Ok(())
}

// ── Komut implementasyonları ───────────────────────────────────────────────────

fn olustur(isim: &str, aciklama: Option<&str>, sessiz: bool) -> Result<()> {
    root_kontrol()?;

    if isim.trim().is_empty() {
        return Err(RollbackError::InvalidInput("Snapshot ismi boş olamaz.".to_string()));
    }

    let backend = detect_backend()?;
    let mut db = SnapshotDB::open(None)?;

    if !sessiz {
        output::bilgi(&format!(
            "Backend: {} | Snapshot oluşturuluyor: \"{}\"",
            backend.name(),
            isim
        ));
    }

    let snap = backend.create(isim, aciklama, SnapshotTrigger::Manual, db.next_id())?;
    let snap_id = snap.id;
    db.add(snap)?;

    if !sessiz {
        output::basari(&format!("Snapshot #{snap_id} oluşturuldu: \"{isim}\""));
    }
    grub_guncelle(sessiz);
    Ok(())
}

fn listele(json: bool) -> Result<()> {
    let db = SnapshotDB::open(None)?;
    let snapshots = db.all();

    if json {
        let out = serde_json::to_string_pretty(snapshots)
            .map_err(RollbackError::Json)?;
        println!("{out}");
    } else {
        output::baslik("RollbackX — Anlık Görüntüler");
        output::snapshot_tablosu_yaz(snapshots);
        println!("Toplam: {} snapshot", snapshots.len());
    }
    Ok(())
}

fn geri_yukle(id: u32, zorla: bool, reboot_yok: bool) -> Result<()> {
    root_kontrol()?;

    let db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;

    if snap.locked {
        return Err(RollbackError::InvalidInput(format!(
            "Snapshot #{id} kilitli — geri yükleme yapılamaz. Önce kilidi kaldırın."
        )));
    }

    output::snapshot_detay_yaz(snap);
    println!();

    if !zorla {
        let onay = output::onay_sor(&format!(
            "Snapshot #{id} \"{}\" geri yüklensin mi? (Sistem yeniden başlatılacak)",
            snap.name
        ));
        if !onay {
            output::bilgi("İptal edildi.");
            return Ok(());
        }
    }

    let backend = detect_backend()?;
    let req = backend.restore(snap)?;

    match req {
        RestoreRequirement::Reboot => {
            if reboot_yok {
                output::uyari(
                    "Geri yükleme için yeniden başlatma gerekiyor. \
                     Lütfen sistemi manuel olarak yeniden başlatın.",
                );
            } else {
                output::basari(&format!(
                    "Snapshot #{id} geri yükleme hazır. Sistem yeniden başlatılıyor..."
                ));
                // update-grub çağır, ardından reboot
                let _ = std::process::Command::new("update-grub").status();
                let _ = std::process::Command::new("reboot").status();
            }
        }
        RestoreRequirement::Immediate => {
            output::basari(&format!("Snapshot #{id} anında geri yüklendi."));
        }
    }
    Ok(())
}

fn sil(id: u32, zorla: bool) -> Result<()> {
    root_kontrol()?;

    let mut db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;

    if snap.locked {
        return Err(RollbackError::InvalidInput(format!(
            "Snapshot #{id} kilitli — silinemez. Önce kilidi kaldırın."
        )));
    }

    if !zorla {
        let onay = output::onay_sor(&format!(
            "Snapshot #{id} \"{}\" kalıcı olarak silinsin mi?",
            snap.name
        ));
        if !onay {
            output::bilgi("İptal edildi.");
            return Ok(());
        }
    }

    let backend = detect_backend()?;
    // snap referansını klonla (db.remove() mut borrow gerektirir)
    let snap_clone = snap.clone();
    backend.delete(&snap_clone)?;
    db.remove(id)?;

    output::basari(&format!("Snapshot #{id} silindi."));
    grub_guncelle(false);
    Ok(())
}

fn bilgi(id: u32) -> Result<()> {
    let db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;
    output::snapshot_detay_yaz(snap);
    Ok(())
}

fn bagla(id: u32) -> Result<()> {
    root_kontrol()?;
    // Reboot sonrası stale mount noktalarını temizle
    cleanup_stale_mount_points();

    let db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;

    let mp = snapshot_mount_point(id);

    if is_snapshot_mounted(&mp) {
        output::bilgi(&format!("Snapshot #{id} zaten bağlı: {}", mp.display()));
        return Ok(());
    }

    let backend = detect_backend()?;
    backend.mount(snap, &mp)?;
    output::basari(&format!("Snapshot #{id} bağlandı: {}", mp.display()));
    Ok(())
}

fn ayir(id: u32) -> Result<()> {
    root_kontrol()?;

    let mp = snapshot_mount_point(id);

    if !is_snapshot_mounted(&mp) {
        // Reboot sonrası stale dizin kalmış olabilir, temizle
        if mp.exists() {
            let _ = std::fs::remove_dir(&mp);
            output::basari(&format!("Snapshot #{id} stale mount noktası temizlendi."));
        } else {
            output::bilgi(&format!("Snapshot #{id} zaten bağlı değil."));
        }
        return Ok(());
    }

    // LVM snapshot ise LV'yi de deaktif et
    {
        let db = SnapshotDB::open(None)?;
        if let Ok(snap) = db.get(id) {
            if snap.backend == rollbackx_core::snapshot::BackendKind::LvmThin {
                unmount_snapshot(&mp)?;
                let lv_path = format!("/dev/{}", snap.backend_ref);
                let _ = std::process::Command::new("lvchange")
                    .args(["-an", &lv_path])
                    .output();
                output::basari(&format!("Snapshot #{id} ayrıldı."));
                return Ok(());
            }
        }
    }

    unmount_snapshot(&mp)?;
    output::basari(&format!("Snapshot #{id} ayrıldı."));
    Ok(())
}

fn kilitle(id: u32) -> Result<()> {
    root_kontrol()?;

    let mut db = SnapshotDB::open(None)?;
    db.set_lock(id, true)?;
    output::basari(&format!("Snapshot #{id} kilitlendi."));
    Ok(())
}

fn kilit_ac(id: u32) -> Result<()> {
    root_kontrol()?;

    let mut db = SnapshotDB::open(None)?;
    db.set_lock(id, false)?;
    output::basari(&format!("Snapshot #{id} kilidi kaldırıldı."));
    Ok(())
}

fn dogrula(id: u32) -> Result<()> {
    root_kontrol()?;

    let db = SnapshotDB::open(None)?;
    let snap = db.get(id)?;

    output::bilgi(&format!(
        "Snapshot #{id} \"{}\" doğrulanıyor...",
        snap.name
    ));

    let backend = detect_backend()?;
    let rapor = backend.verify(snap)?;

    println!();
    for k in &rapor.kontroller {
        let isaret = if k.basarili { "✓" } else { "✗" };
        println!("  {isaret} {} — {}", k.ad, k.mesaj);
    }
    println!();

    if rapor.tumu_basarili {
        output::basari("Doğrulama başarılı: tüm sistem dosyaları sağlıklı.");
    } else {
        let n = rapor.kontroller.iter().filter(|k| !k.basarili).count();
        output::uyari(&format!("Doğrulama tamamlandı: {n} kontrol başarısız."));
    }
    Ok(())
}

fn yeniden_adlandir(id: u32, isim: String, aciklama: Option<String>) -> Result<()> {
    root_kontrol()?;

    if isim.trim().is_empty() {
        return Err(RollbackError::InvalidInput("Snapshot ismi boş olamaz.".to_string()));
    }

    let mut db = SnapshotDB::open(None)?;
    let eski_isim = db.get(id)?.name.clone();
    db.rename(id, isim.clone(), aciklama)?;
    output::basari(&format!("Snapshot #{id} yeniden adlandırıldı: \"{eski_isim}\" → \"{isim}\""));
    Ok(())
}

/// update-grub varsa sessizce çalıştır
fn grub_guncelle(sessiz: bool) {
    if !sessiz {
        output::bilgi("GRUB menüsü güncelleniyor...");
    }
    // which ile tam yolu bul, yoksa direkt dene
    let cmd = std::process::Command::new("which")
        .arg("update-grub")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "update-grub".to_string());

    let _ = std::process::Command::new(&cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn durum() -> Result<()> {
    output::baslik("RollbackX — Sistem Durumu");

    let backend_sonuc = detect_backend();
    match &backend_sonuc {
        Ok(backend) => output::bilgi(&format!("Backend:  {}", backend.name())),
        Err(e)      => output::uyari(&format!("Backend:  kullanılamıyor — {e}")),
    }

    match SnapshotDB::open(None) {
        Ok(db) => {
            let snapshots = db.all();
            output::bilgi(&format!("Snapshot: {} adet", snapshots.len()));
            let toplam: u64 = snapshots.iter().filter_map(|s| s.size_bytes).sum();
            if toplam > 0 {
                output::bilgi(&format!("Toplam boyut: {}", output::boyut_format(toplam)));
            }
        }
        Err(e) => output::uyari(&format!("DB okunamadı: {e}")),
    }
    Ok(())
}
