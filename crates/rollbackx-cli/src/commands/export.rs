use std::path::{Path, PathBuf};

use chrono::Local;
use clap::Subcommand;
use colored::Colorize;

use rollbackx_core::error::Result;
use rollbackx_core::{export, import, SnapshotDB, SnapshotTrigger, Snapshot};

use crate::output;

#[derive(Subcommand)]
pub enum ExportKomut {
    /// Snapshot'ı .rxsnap dosyasına dışa aktar
    #[command(name = "disa-aktar", alias = "export")]
    DisaAktar {
        /// Snapshot ID
        id: u32,
        /// Çıktı dosyası (varsayılan: rollbackx_<isim>_<tarih>.rxsnap)
        #[arg(short = 'o', long = "cikti")]
        cikti: Option<PathBuf>,
    },
    /// .rxsnap dosyasını sisteme içe aktar
    #[command(name = "ice-aktar", alias = "import")]
    IceAktar {
        /// .rxsnap dosya yolu
        dosya: PathBuf,
    },
    /// .rxsnap dosyasının içeriğini göster (açmadan)
    #[command(name = "bilgi", alias = "info")]
    Bilgi {
        /// .rxsnap dosya yolu
        dosya: PathBuf,
    },
}

pub fn calistir(komut: ExportKomut) -> Result<()> {
    match komut {
        ExportKomut::DisaAktar { id, cikti } => disa_aktar(id, cikti),
        ExportKomut::IceAktar  { dosya }     => ice_aktar(dosya),
        ExportKomut::Bilgi     { dosya }     => bilgi(dosya),
    }
}

fn disa_aktar(id: u32, cikti: Option<PathBuf>) -> Result<()> {
    let db       = SnapshotDB::open(None)?;
    let snapshot = db.get(id)?;

    let out_path = cikti.unwrap_or_else(|| {
        let ts   = Local::now().format("%Y%m%d");
        let name = snapshot.name.replace(' ', "_");
        PathBuf::from(format!("rollbackx_{}_{}.rxsnap", name, ts))
    });

    println!(
        "{} {} → {}",
        "→".green().bold(),
        format!("Snapshot #{} dışa aktarılıyor...", id).bold(),
        out_path.display().to_string().cyan()
    );
    println!(
        "  Backend : {}",
        snapshot.backend.to_string().yellow()
    );

    export(snapshot, &out_path)?;

    let size = std::fs::metadata(&out_path)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".to_string());

    println!(
        "{} Dışa aktarıldı: {} ({})",
        "✓".green().bold(),
        out_path.display().to_string().cyan().bold(),
        size.yellow()
    );
    println!("  Başka bir sisteme taşıyabilir ve içe aktarabilirsiniz.");
    Ok(())
}

fn ice_aktar(dosya: PathBuf) -> Result<()> {
    if !dosya.exists() {
        output::hata_yaz(&format!("Dosya bulunamadı: {}", dosya.display()));
        return Ok(());
    }

    println!(
        "{} {} dosyası içe aktarılıyor...",
        "→".green().bold(),
        dosya.display().to_string().cyan()
    );

    let target_base = Path::new("/var/lib/rollbackx/snapshots");
    let (meta, dest) = import(&dosya, target_base)?;

    // Snapshot'ı DB'ye kaydet
    let mut db  = SnapshotDB::open(None)?;
    let new_id  = db.next_id();
    let created = chrono::DateTime::parse_from_rfc3339(&meta.created_at)
        .map(|dt| dt.with_timezone(&chrono::Local))
        .unwrap_or_else(|_| chrono::Local::now());

    // backend_ref: import()'ın gerçekte oluşturduğu dizinin adı (tam yol değil)
    let backend_ref = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let snapshot = Snapshot {
        id:          new_id,
        name:        format!("{} (içe aktarıldı)", meta.name),
        description: meta.description,
        created_at:  created,
        trigger:     SnapshotTrigger::Manual,
        backend:     meta.backend.clone(),
        backend_ref,
        size_bytes:  None,
        locked:      false,
    };
    db.add(snapshot)?;

    println!(
        "{} İçe aktarıldı: \"{}\" → ID #{}",
        "✓".green().bold(),
        meta.name.bold(),
        new_id.to_string().cyan()
    );
    println!(
        "  Kaynak    : {} ({})",
        meta.hostname.yellow(),
        meta.exported_at
    );
    println!("  Backend   : {}", meta.backend.to_string().yellow());
    Ok(())
}

fn bilgi(dosya: PathBuf) -> Result<()> {
    use std::process::Command;
    use rollbackx_core::error::RollbackError;

    if !dosya.exists() {
        output::hata_yaz(&format!("Dosya bulunamadı: {}", dosya.display()));
        return Ok(());
    }

    // Arşivden sadece metadata.json çıkar
    let tmp = tempfile::TempDir::new()
        .map_err(RollbackError::Io)?;
    let status = Command::new("tar")
        .args([
            "--use-compress-program=zstd",
            "-xf", &dosya.to_string_lossy(),
            "-C", &tmp.path().to_string_lossy(),
            "./metadata.json",
        ])
        .status()
        .map_err(RollbackError::Io)?;

    if !status.success() {
        output::hata_yaz("Geçersiz .rxsnap dosyası");
        return Ok(());
    }

    let meta_str = std::fs::read_to_string(tmp.path().join("metadata.json"))?;
    let meta: rollbackx_core::RxSnapMeta = serde_json::from_str(&meta_str)?;

    let size = std::fs::metadata(&dosya)
        .map(|m| format_bytes(m.len()))
        .unwrap_or_else(|_| "?".to_string());

    println!("{}", "─── .rxsnap Dosya Bilgisi ─────────────────────".bright_black());
    println!("  Dosya      : {}", dosya.display().to_string().cyan());
    println!("  Boyut      : {}", size.yellow());
    println!("  Snapshot   : \"{}\" (ID #{})", meta.name.bold(), meta.snapshot_id);
    println!("  Backend    : {}", meta.backend.to_string().yellow());
    println!("  Oluşturuldu: {}", meta.created_at);
    println!("  Kaynak     : {}", meta.hostname.yellow());
    println!("  Dışa aktarıldı: {}", meta.exported_at);
    println!("  RX Sürümü : {}", meta.rx_version);
    if let Some(desc) = &meta.description {
        println!("  Açıklama   : {}", desc);
    }
    println!("{}", "───────────────────────────────────────────────".bright_black());
    Ok(())
}

fn format_bytes(b: u64) -> String {
    match b {
        b if b < 1024               => format!("{b} B"),
        b if b < 1024 * 1024       => format!("{:.1} KB", b as f64 / 1024.0),
        b if b < 1024 * 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b => format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}
