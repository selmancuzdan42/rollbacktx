use clap::Subcommand;
use colored::Colorize;

use rollbackx_core::error::{Result, RollbackError};

const HOOK_SRC: &str = "/usr/share/rollbackx/apt-hook/80rollbackx";
const HOOK_DST: &str = "/etc/apt/apt.conf.d/80rollbackx";

#[derive(Subcommand)]
pub enum AyarKomut {
    /// APT hook yönetimi (otomatik snapshot)
    #[command(name = "apt-hook")]
    AptHook {
        #[command(subcommand)]
        islem: AptHookIslem,
    },
}

#[derive(Subcommand)]
pub enum AptHookIslem {
    /// APT hook'unu etkinleştir
    #[command(name = "aç", alias = "ac")]
    Ac,
    /// APT hook'unu devre dışı bırak
    #[command(name = "kapat")]
    Kapat,
    /// APT hook durumunu göster
    #[command(name = "durum")]
    Durum,
}

pub fn calistir(komut: AyarKomut) -> Result<()> {
    match komut {
        AyarKomut::AptHook { islem } => apt_hook(islem),
    }
}

fn apt_hook(islem: AptHookIslem) -> Result<()> {
    match islem {
        AptHookIslem::Ac => {
            if std::path::Path::new(HOOK_DST).exists() {
                println!("{} APT hook zaten etkin.", "ℹ".cyan());
                return Ok(());
            }
            if !std::path::Path::new(HOOK_SRC).exists() {
                return Err(RollbackError::InvalidInput(format!(
                    "Hook şablonu bulunamadı: {HOOK_SRC}"
                )));
            }
            std::fs::copy(HOOK_SRC, HOOK_DST).map_err(RollbackError::Io)?;
            println!(
                "{} APT hook etkinleştirildi.",
                "✓".green().bold()
            );
            println!("  Artık her 'apt install/upgrade' öncesinde otomatik snapshot alınacak.");
        }
        AptHookIslem::Kapat => {
            if !std::path::Path::new(HOOK_DST).exists() {
                println!("{} APT hook zaten devre dışı.", "ℹ".cyan());
                return Ok(());
            }
            std::fs::remove_file(HOOK_DST).map_err(RollbackError::Io)?;
            println!(
                "{} APT hook devre dışı bırakıldı.",
                "✓".green().bold()
            );
        }
        AptHookIslem::Durum => {
            let aktif = std::path::Path::new(HOOK_DST).exists();
            let durum = if aktif {
                "etkin".green().bold()
            } else {
                "devre dışı".red().bold()
            };
            println!("APT hook: {durum}");
            if aktif {
                println!("  Konum : {HOOK_DST}");
                println!("  Her 'apt install/upgrade' öncesinde snapshot alınıyor.");
            } else {
                println!("  Etkinleştirmek için: rollbackx ayarlar apt-hook aç");
            }
        }
    }
    Ok(())
}
