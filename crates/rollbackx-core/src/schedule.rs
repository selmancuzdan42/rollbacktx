use chrono::{Datelike, Local, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fmt;

use crate::error::{Result, RollbackError};

const SCHEDULE_PATH: &str = "/var/lib/rollbackx/schedule.json";

// ── Sıklık ───────────────────────────────────────────────────────────────────

/// Zamanlayıcı tekrar sıklığı.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Siklik {
    /// Her gün tetiklenir.
    Gunluk,
    /// Haftanın belirli bir gününde tetiklenir.
    Haftalik,
    /// Ayın belirli bir haftasının belirli gününde tetiklenir.
    Aylik,
}

impl fmt::Display for Siklik {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gunluk   => write!(f, "Günlük"),
            Self::Haftalik => write!(f, "Haftalık"),
            Self::Aylik    => write!(f, "Aylık"),
        }
    }
}

impl std::str::FromStr for Siklik {
    type Err = RollbackError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "gunluk"   | "günlük"   | "daily"   => Ok(Self::Gunluk),
            "haftalik" | "haftalık" | "weekly"  => Ok(Self::Haftalik),
            "aylik"    | "aylık"    | "monthly" => Ok(Self::Aylik),
            _ => Err(RollbackError::InvalidInput(
                format!("Geçersiz sıklık: '{s}'. Değerler: gunluk, haftalik, aylik")
            )),
        }
    }
}

// ── Gün ──────────────────────────────────────────────────────────────────────

/// Haftanın günleri — zamanlayıcı yapılandırmasında kullanılır.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleGun {
    Pazartesi,
    Sali,
    Carsamba,
    Persembe,
    Cuma,
    Cumartesi,
    Pazar,
}

impl ScheduleGun {
    pub fn to_chrono(&self) -> Weekday {
        match self {
            Self::Pazartesi => Weekday::Mon,
            Self::Sali      => Weekday::Tue,
            Self::Carsamba  => Weekday::Wed,
            Self::Persembe  => Weekday::Thu,
            Self::Cuma      => Weekday::Fri,
            Self::Cumartesi => Weekday::Sat,
            Self::Pazar     => Weekday::Sun,
        }
    }
}

impl fmt::Display for ScheduleGun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pazartesi => write!(f, "Pazartesi"),
            Self::Sali      => write!(f, "Salı"),
            Self::Carsamba  => write!(f, "Çarşamba"),
            Self::Persembe  => write!(f, "Perşembe"),
            Self::Cuma      => write!(f, "Cuma"),
            Self::Cumartesi => write!(f, "Cumartesi"),
            Self::Pazar     => write!(f, "Pazar"),
        }
    }
}

impl std::str::FromStr for ScheduleGun {
    type Err = RollbackError;
    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().replace('ı', "i").replace('ş', "s").as_str() {
            "pazartesi" | "mon" | "monday"    => Ok(Self::Pazartesi),
            "sali" | "salı" | "tue" | "tuesday"   => Ok(Self::Sali),
            "carsamba" | "çarşamba" | "wed" | "wednesday" => Ok(Self::Carsamba),
            "persembe" | "perşembe" | "thu" | "thursday"  => Ok(Self::Persembe),
            "cuma" | "fri" | "friday"         => Ok(Self::Cuma),
            "cumartesi" | "sat" | "saturday"  => Ok(Self::Cumartesi),
            "pazar" | "sun" | "sunday"        => Ok(Self::Pazar),
            _ => Err(RollbackError::InvalidInput(
                format!("Geçersiz gün: '{s}'")
            )),
        }
    }
}

// ── Schedule ─────────────────────────────────────────────────────────────────

/// Zamanlanmış geri yükleme yapılandırması.
///
/// `/var/lib/rollbackx/schedule.json` dosyasında saklanır.
/// Systemd timer tarafından periyodik olarak kontrol edilir.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub enabled:     bool,
    pub snapshot_id: u32,
    pub siklik:      Siklik,
    /// Haftalık/aylık için gün
    pub gun:         Option<ScheduleGun>,
    /// Aylık için kaçıncı hafta (1=ilk, 2=ikinci, 3=üçüncü, 4=dördüncü)
    pub kacinci:     Option<u8>,
    /// Geri yükleme saati "HH:MM" formatında
    pub saat:        String,
}

impl Schedule {
    /// Zamanlayıcı yapılandırma dosyasının yolunu döndürür.
    pub fn yol() -> PathBuf {
        PathBuf::from(SCHEDULE_PATH)
    }

    /// Zamanlayıcı yapılandırmasını dosyadan yükler. Dosya yoksa `None` döner.
    pub fn yukle() -> Result<Option<Self>> {
        let yol = Self::yol();
        if !yol.exists() {
            return Ok(None);
        }
        let icerik = std::fs::read_to_string(&yol)
            .map_err(RollbackError::Io)?;
        if icerik.trim().is_empty() {
            return Ok(None);
        }
        let s: Schedule = serde_json::from_str(&icerik)
            .map_err(RollbackError::Json)?;
        Ok(Some(s))
    }

    /// Yapılandırmayı diske kaydeder.
    pub fn kaydet(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(RollbackError::Json)?;
        std::fs::write(Self::yol(), json)
            .map_err(RollbackError::Io)?;
        Ok(())
    }

    /// Zamanlayıcı yapılandırmasını siler.
    pub fn kaldir() -> Result<()> {
        let yol = Self::yol();
        if yol.exists() {
            std::fs::remove_file(&yol).map_err(RollbackError::Io)?;
        }
        Ok(())
    }

    /// Şu an verilmiş zaman damgasına göre bu zamanlayıcı tetiklenmeli mi?
    pub fn tetikle_mi(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let bugun = Local::now();
        match &self.siklik {
            Siklik::Gunluk => true,
            Siklik::Haftalik => {
                if let Some(gun) = &self.gun {
                    bugun.weekday() == gun.to_chrono()
                } else {
                    false
                }
            }
            Siklik::Aylik => {
                if let Some(gun) = &self.gun {
                    let n = self.kacinci.unwrap_or(1);
                    let hedef_gun = aylik_gun_tarihi(
                        bugun.year(),
                        bugun.month(),
                        gun.to_chrono(),
                        n,
                    );
                    match hedef_gun {
                        Some(tarih) => tarih == bugun.day(),
                        None => false,
                    }
                } else {
                    false
                }
            }
        }
    }

    /// İnsan okunabilir açıklama
    pub fn acikla(&self) -> String {
        match &self.siklik {
            Siklik::Gunluk => format!("Her gün saat {}", self.saat),
            Siklik::Haftalik => {
                let gun = self.gun.as_ref()
                    .map(|g| g.to_string())
                    .unwrap_or_else(|| "?".into());
                format!("Her hafta {} saat {}", gun, self.saat)
            }
            Siklik::Aylik => {
                let gun = self.gun.as_ref()
                    .map(|g| g.to_string())
                    .unwrap_or_else(|| "?".into());
                let kacinci = match self.kacinci.unwrap_or(1) {
                    1 => "ilk",
                    2 => "ikinci",
                    3 => "üçüncü",
                    4 => "dördüncü",
                    _ => "?",
                };
                format!("Her ayın {} {} günü saat {}", kacinci, gun, self.saat)
            }
        }
    }
}

/// Belirtilen ay/yılın N'inci WEEKDAY gününün ayın kaçı olduğunu döndürür.
/// Örnek: `aylik_gun_tarihi(2026, 2, Mon, 1)` → `Some(2)` (Şubat 2026'nın ilk Pazartesi'si 2'sidir)
pub fn aylik_gun_tarihi(yil: i32, ay: u32, hedef: Weekday, n: u8) -> Option<u32> {
    let ayin_ilki = NaiveDate::from_ymd_opt(yil, ay, 1)?;
    let ilk_gun   = ayin_ilki.weekday();

    // Hedef günün bu ayda ilk hangi tarihte olduğunu bul
    let fark = (hedef.num_days_from_monday() as i32
        - ilk_gun.num_days_from_monday() as i32 + 7) % 7;
    let ilk_tarih = 1 + fark as u32;
    let nth_tarih = ilk_tarih + (n as u32 - 1) * 7;

    // Ay sınırını aşmıyor mu?
    NaiveDate::from_ymd_opt(yil, ay, nth_tarih)?;
    Some(nth_tarih)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ilk_pazartesi_subat_2026() {
        // Şubat 2026: 1. Pazartesi = 2 Şubat
        assert_eq!(aylik_gun_tarihi(2026, 2, Weekday::Mon, 1), Some(2));
    }

    #[test]
    fn test_ucuncu_cuma_ocak_2026() {
        // Ocak 2026: 3. Cuma = 16 Ocak
        assert_eq!(aylik_gun_tarihi(2026, 1, Weekday::Fri, 3), Some(16));
    }

    #[test]
    fn test_besinci_pazartesi_olmayan_ay() {
        // Şubat 2026'nın 5. Pazartesi'si yok (sadece 4 hafta)
        assert_eq!(aylik_gun_tarihi(2026, 2, Weekday::Mon, 5), None);
    }

    #[test]
    fn test_siklik_parse() {
        assert_eq!("gunluk".parse::<Siklik>().unwrap(), Siklik::Gunluk);
        assert_eq!("haftalik".parse::<Siklik>().unwrap(), Siklik::Haftalik);
        assert_eq!("monthly".parse::<Siklik>().unwrap(), Siklik::Aylik);
    }

    #[test]
    fn test_gun_parse() {
        assert_eq!("pazartesi".parse::<ScheduleGun>().unwrap(), ScheduleGun::Pazartesi);
        assert_eq!("friday".parse::<ScheduleGun>().unwrap(), ScheduleGun::Cuma);
    }

    #[test]
    fn test_gunluk_tetikle() {
        let s = Schedule {
            enabled:     true,
            snapshot_id: 1,
            siklik:      Siklik::Gunluk,
            gun:         None,
            kacinci:     None,
            saat:        "03:00".to_string(),
        };
        // Günlük her zaman tetiklenmeli
        assert!(s.tetikle_mi());
    }

    #[test]
    fn test_acikla_aylik() {
        let s = Schedule {
            enabled:     true,
            snapshot_id: 1,
            siklik:      Siklik::Aylik,
            gun:         Some(ScheduleGun::Pazartesi),
            kacinci:     Some(1),
            saat:        "03:00".to_string(),
        };
        assert_eq!(s.acikla(), "Her ayın ilk Pazartesi günü saat 03:00");
    }
}
