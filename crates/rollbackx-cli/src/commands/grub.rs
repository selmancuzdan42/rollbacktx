use clap::Subcommand;
use rollbackx_core::error::{Result, RollbackError};
use crate::output;

#[derive(Subcommand)]
pub enum GrubKomut {
    /// GRUB menüsünü snapshot'lara göre güncelle
    #[command(name = "güncelle", alias = "guncelle")]
    Guncelle,
}

pub fn calistir(komut: GrubKomut) -> Result<()> {
    match komut {
        GrubKomut::Guncelle => guncelle(),
    }
}

fn guncelle() -> Result<()> {
    output::bilgi("GRUB menüsü güncelleniyor...");

    let status = std::process::Command::new("update-grub")
        .status()
        .map_err(|e| RollbackError::CommandFailed {
            cmd: "update-grub".to_string(),
            reason: e.to_string(),
        })?;

    if status.success() {
        output::basari("GRUB menüsü başarıyla güncellendi.");
    } else {
        return Err(RollbackError::CommandFailed {
            cmd: "update-grub".to_string(),
            reason: format!("Çıkış kodu: {:?}", status.code()),
        });
    }
    Ok(())
}
