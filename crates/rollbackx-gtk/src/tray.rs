//! Sistem tepsisi (StatusNotifierItem) — ksni ile pure-Rust implementasyon.

use ksni::{menu::*, MenuItem, Tray};

/// Tepsi simgesine tıklandığında veya menü seçildiğinde
/// GTK ana thread'e gönderilecek komutlar.
#[derive(Debug, Clone)]
pub enum TrayCmd {
    /// Pencereyi göster
    Show,
    /// Yeni snapshot dialog'unu aç
    NewSnapshot,
    /// Uygulamadan çık
    Quit,
}

pub struct RollbackxTray {
    pub sender: std::sync::mpsc::Sender<TrayCmd>,
}

impl Tray for RollbackxTray {
    fn id(&self) -> String {
        "rollbackx".into()
    }

    fn icon_name(&self) -> String {
        "camera-photo-symbolic".into()
    }

    fn title(&self) -> String {
        "RollbackX".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "camera-photo-symbolic".into(),
            icon_pixmap: vec![],
            title: "RollbackX".into(),
            description: "Sistem anlık görüntüsü yöneticisi".into(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.sender.send(TrayCmd::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "RollbackX'i Aç".into(),
                icon_name: "window-restore-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCmd::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Yeni Snapshot".into(),
                icon_name: "list-add-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCmd::NewSnapshot);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Çıkış".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|tray: &mut Self| {
                    let _ = tray.sender.send(TrayCmd::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Tray'i ayrı thread'de başlat, komutları `sender` üzerinden gönderir.
pub fn spawn_tray(sender: std::sync::mpsc::Sender<TrayCmd>) {
    std::thread::spawn(move || {
        let tray = RollbackxTray { sender };
        ksni::TrayService::new(tray).spawn();
    });
}
