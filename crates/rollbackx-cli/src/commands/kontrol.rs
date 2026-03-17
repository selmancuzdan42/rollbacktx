use colored::Colorize;
use rollbackx_core::{backend::detect_backend, error::Result, snapshot::SnapshotDB};

use crate::output;

pub fn kontrol_et() -> Result<()> {
    output::baslik("RollbackX — Sistem Kontrolü");
    println!();

    let mut tamam = true;

    // ── Backend ───────────────────────────────────────────────────────────────
    match detect_backend() {
        Ok(b)  => ok(&format!("Backend          {}", b.name())),
        Err(e) => { hata(&format!("Backend          {e}")); tamam = false; }
    }

    // ── Snapshot DB ───────────────────────────────────────────────────────────
    match SnapshotDB::open(None) {
        Ok(db) => ok(&format!("Snapshot DB      {} adet kayıt", db.all().len())),
        Err(e) => { hata(&format!("Snapshot DB      {e}")); tamam = false; }
    }

    // ── GRUB scripti ─────────────────────────────────────────────────────────
    if std::path::Path::new("/etc/grub.d/80_rollbackx").exists() {
        ok("GRUB scripti     /etc/grub.d/80_rollbackx");
    } else {
        uyar("GRUB scripti     bulunamadı (rollbackx kurulu değil mi?)");
        tamam = false;
    }

    // ── update-grub ───────────────────────────────────────────────────────────
    if komut_var("update-grub") {
        ok("update-grub      mevcut");
    } else {
        uyar("update-grub      bulunamadı (grub-common kurulu değil mi?)");
    }

    // ── rsync ─────────────────────────────────────────────────────────────────
    if komut_var("rsync") {
        ok("rsync            mevcut");
    } else {
        hata("rsync            kurulu değil!");
        tamam = false;
    }

    // ── python3 ───────────────────────────────────────────────────────────────
    if komut_var("python3") {
        ok("python3          mevcut");
    } else {
        uyar("python3          bulunamadı (GRUB scripti çalışmayabilir)");
    }

    // ── pkexec ────────────────────────────────────────────────────────────────
    if komut_var("pkexec") {
        ok("pkexec           mevcut");
    } else {
        uyar("pkexec           bulunamadı (GUI yetkilendirmesi çalışmaz)");
    }

    // ── Systemd servisleri ────────────────────────────────────────────────────
    for servis in &["rollbackx-restore.service", "rollbackx-cmdline.service"] {
        let durum = std::process::Command::new("systemctl")
            .args(["is-enabled", servis])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "bilinmiyor".to_string());

        match durum.as_str() {
            "enabled" | "static" => ok(&format!("systemd          {servis} → etkin")),
            "disabled"           => uyar(&format!("systemd          {servis} → devre dışı")),
            other                => uyar(&format!("systemd          {servis} → {other}")),
        }
    }

    // ── Disk kullanımı ────────────────────────────────────────────────────────
    disk_kontrol(&mut tamam);

    // ── Özet ─────────────────────────────────────────────────────────────────
    println!();
    if tamam {
        output::basari("Tüm kontroller geçti — sistem hazır.");
    } else {
        output::uyari("Bazı kontroller başarısız. Yukarıdaki uyarıları inceleyin.");
    }

    Ok(())
}

fn disk_kontrol(tamam: &mut bool) {
    let Ok(cikti) = std::process::Command::new("df")
        .args(["-h", "/var/lib/rollbackx"])
        .output()
    else {
        return;
    };

    let txt = String::from_utf8_lossy(&cikti.stdout);
    for line in txt.lines().skip(1) {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() >= 5 {
            let pct_str = p[4];
            let pct: u32 = pct_str.trim_end_matches('%').parse().unwrap_or(0);
            let nokta = p.get(5).copied().unwrap_or("/");
            let msg   = format!("Disk ({nokta})    {pct_str} dolu — {} boş", p[3]);
            if pct >= 90 { hata(&msg); *tamam = false; }
            else if pct >= 75 { uyar(&msg); }
            else { ok(&msg); }
        }
    }
}

fn komut_var(komut: &str) -> bool {
    std::process::Command::new("which")
        .arg(komut)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ok(msg: &str)   { println!("  {} {}", "✓".green().bold(), msg); }
fn uyar(msg: &str) { println!("  {} {}", "⚠".yellow().bold(), msg.yellow()); }
fn hata(msg: &str) { println!("  {} {}", "✗".red().bold(), msg.red()); }
