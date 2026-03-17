use colored::Colorize;
use rollbackx_core::snapshot::Snapshot;
use tabled::{Table, Tabled};

/// Ortam değişkenine göre Türkçe mi İngilizce mi?
pub fn dil_en() -> bool {
    std::env::var("ROLLBACKX_LANG")
        .map(|v| v.to_lowercase() == "en")
        .unwrap_or(false)
}

pub fn baslik(msg: &str) {
    println!("{}", msg.bold().cyan());
}

pub fn basari(msg: &str) {
    println!("{} {}", "✓".green().bold(), msg.green());
}

pub fn uyari(msg: &str) {
    println!("{} {}", "⚠".yellow().bold(), msg.yellow());
}

pub fn hata_yaz(msg: &str) {
    eprintln!("{} {}", "✗".red().bold(), msg.red());
}

pub fn bilgi(msg: &str) {
    println!("{} {}", "→".blue(), msg);
}

/// Bayt değerini insan okunabilir formata çevirir
pub fn boyut_format(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b if b < 1024 * 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b => format!("{:.2} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}

pub fn onay_sor(soru: &str) -> bool {
    use std::io::{self, Write};
    print!("{} [e/H]: ", soru.yellow());
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "e" | "evet" | "y" | "yes")
}

// ── Snapshot tablosu ──────────────────────────────────────────────────────────

#[derive(Tabled)]
struct SnapshotSatir {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "İsim")]
    isim: String,
    #[tabled(rename = "Tarih")]
    tarih: String,
    #[tabled(rename = "Tetikleyici")]
    tetikleyici: String,
    #[tabled(rename = "Backend")]
    backend: String,
    #[tabled(rename = "Boyut")]
    boyut: String,
}

pub fn snapshot_tablosu_yaz(snapshots: &[Snapshot]) {
    if snapshots.is_empty() {
        uyari(if dil_en() {
            "No snapshots found."
        } else {
            "Hiç snapshot bulunamadı."
        });
        return;
    }

    let satirlar: Vec<SnapshotSatir> = snapshots
        .iter()
        .map(|s| SnapshotSatir {
            id:          s.id.to_string(),
            isim:        s.name.clone(),
            tarih:       s.created_at.format("%Y-%m-%d %H:%M").to_string(),
            tetikleyici: s.trigger.to_string(),
            backend:     s.backend.to_string(),
            boyut:       s.size_human(),
        })
        .collect();

    let tablo = Table::new(satirlar).to_string();
    println!("{tablo}");
}

pub fn snapshot_detay_yaz(s: &Snapshot) {
    let baslik_str = if dil_en() {
        format!("Snapshot #{}", s.id)
    } else {
        format!("Anlık Görüntü #{}", s.id)
    };
    baslik(&baslik_str);

    let etiketler: &[(&str, &str)] = if dil_en() {
        &[
            ("Name",        ""),
            ("Description", ""),
            ("Created",     ""),
            ("Trigger",     ""),
            ("Backend",     ""),
            ("Ref",         ""),
            ("Size",        ""),
        ]
    } else {
        &[
            ("İsim",         ""),
            ("Açıklama",     ""),
            ("Oluşturuldu",  ""),
            ("Tetikleyici",  ""),
            ("Backend",      ""),
            ("Referans",     ""),
            ("Boyut",        ""),
        ]
    };

    let degerler = [
        s.name.as_str(),
        s.description.as_deref().unwrap_or("—"),
        &s.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        &s.trigger.to_string(),
        &s.backend.to_string(),
        &s.backend_ref,
        &s.size_human(),
    ];

    for (i, (etiket, _)) in etiketler.iter().enumerate() {
        println!("  {:15} {}", format!("{etiket}:").bold(), degerler[i]);
    }
}
