mod commands;
mod output;

use clap::{Parser, Subcommand};
use std::process;

/// RollbackX — Pardus Linux sistem anlık görüntüsü yöneticisi
#[derive(Parser)]
#[command(
    name = "rollbackx",
    version,
    about = "Pardus Linux için Btrfs/LVM Thin sistem snapshot yöneticisi",
    long_about = None,
    arg_required_else_help = true,
)]
struct Cli {
    #[command(subcommand)]
    command: Komut,
}

#[derive(Subcommand)]
enum Komut {
    /// Snapshot işlemleri
    #[command(name = "snapshot", alias = "snap")]
    Snapshot {
        #[command(subcommand)]
        alt_komut: commands::snapshot::SnapKomut,
    },
    /// GRUB menüsünü güncelle
    #[command(name = "grub")]
    Grub {
        #[command(subcommand)]
        alt_komut: commands::grub::GrubKomut,
    },
    /// Sistem durumu: backend, snapshot sayısı, disk kullanımı
    #[command(name = "durum")]
    Durum,
    /// Sistem kontrolü: backend, servisler, disk, bağımlılıklar
    #[command(name = "kontrol", alias = "check")]
    Kontrol,
    /// Zamanlanmış otomatik geri yükleme yönetimi
    #[command(name = "zamanlayici", alias = "schedule")]
    Zamanlayici {
        #[command(subcommand)]
        alt_komut: commands::zamanlayici::ZamanKomut,
    },
    /// Snapshot'ları .rxsnap dosyası olarak dışa/içe aktar
    #[command(name = "arsiv", alias = "archive")]
    Arsiv {
        #[command(subcommand)]
        alt_komut: commands::export::ExportKomut,
    },
    /// Uygulama ayarları (APT hook vb.)
    #[command(name = "ayarlar", alias = "settings")]
    Ayarlar {
        #[command(subcommand)]
        alt_komut: commands::ayarlar::AyarKomut,
    },
    /// Her açılışta otomatik geri yükleme
    #[command(name = "oto")]
    Oto {
        #[command(subcommand)]
        alt_komut: commands::oto::OtoKomut,
    },
}

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        output::hata_yaz(&e.to_string());
        process::exit(1);
    }
}

fn run(cli: Cli) -> rollbackx_core::error::Result<()> {
    match cli.command {
        Komut::Snapshot { alt_komut } => {
            commands::snapshot::calistir(alt_komut)
        }
        Komut::Grub { alt_komut } => {
            commands::grub::calistir(alt_komut)
        }
        Komut::Durum => {
            commands::snapshot::durum()
        }
        Komut::Kontrol => {
            commands::kontrol::kontrol_et()
        }
        Komut::Zamanlayici { alt_komut } => {
            commands::zamanlayici::calistir(alt_komut)
        }
        Komut::Arsiv { alt_komut } => {
            commands::export::calistir(alt_komut)
        }
        Komut::Ayarlar { alt_komut } => {
            commands::ayarlar::calistir(alt_komut)
        }
        Komut::Oto { alt_komut } => {
            commands::oto::calistir(alt_komut)
        }
    }
}
