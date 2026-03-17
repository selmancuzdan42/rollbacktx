//! Snapshot liste satırı widget'ı.

use gtk4::prelude::*;
use rollbackx_core::snapshot::{BackendKind, Snapshot, SnapshotTrigger};

fn relative_time(dt: &chrono::DateTime<chrono::Local>) -> String {
    let secs = chrono::Local::now()
        .signed_duration_since(*dt)
        .num_seconds()
        .max(0);
    match secs {
        s if s < 60              => "Az önce".to_string(),
        s if s < 3600            => format!("{} dakika önce", s / 60),
        s if s < 86400           => format!("{} saat önce", s / 3600),
        s if s < 86400 * 7       => format!("{} gün önce", s / 86400),
        s if s < 86400 * 30      => format!("{} hafta önce", s / (86400 * 7)),
        s if s < 86400 * 365     => format!("{} ay önce", s / (86400 * 30)),
        s                        => format!("{} yıl önce", s / (86400 * 365)),
    }
}

/// Snapshot listesindeki tek bir satır.
pub struct SnapshotRow {
    row: gtk4::ListBoxRow,
}

impl SnapshotRow {
    pub fn new(snap: &Snapshot) -> Self {
        let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        hbox.set_margin_top(10);
        hbox.set_margin_bottom(10);
        hbox.set_margin_start(12);
        hbox.set_margin_end(12);

        // ── Backend ikonu ────────────────────────────────────────────────────
        let icon_name = match snap.backend {
            BackendKind::Btrfs   => "drive-harddisk-symbolic",
            BackendKind::LvmThin => "media-flash-symbolic",
            BackendKind::Rsync   => "folder-saved-search-symbolic",
        };
        let icon = gtk4::Image::builder()
            .icon_name(icon_name)
            .icon_size(gtk4::IconSize::Large)
            .build();
        hbox.append(&icon);

        // ── Bilgi kutusu (dikey) ─────────────────────────────────────────────
        let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        vbox.set_hexpand(true);

        let lbl_name = gtk4::Label::builder()
            .label(&snap.name)
            .xalign(0.0)
            .css_classes(["title-4"])
            .build();

        // İkinci satır: relative time + trigger
        let trigger_str = match &snap.trigger {
            SnapshotTrigger::Manual              => "El ile".to_string(),
            SnapshotTrigger::Apt { packages }    => {
                if packages.is_empty() { "APT".to_string() }
                else { format!("APT ({})", packages.join(", ")) }
            }
            SnapshotTrigger::Cron                => "Zamanlayıcı".to_string(),
        };
        let detail = format!(
            "{} · {} · {}",
            relative_time(&snap.created_at),
            trigger_str,
            snap.created_at.format("%Y-%m-%d %H:%M"),
        );
        let lbl_detail = gtk4::Label::builder()
            .label(&detail)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();

        vbox.append(&lbl_name);
        vbox.append(&lbl_detail);

        if let Some(desc) = &snap.description {
            if !desc.is_empty() {
                let lbl_desc = gtk4::Label::builder()
                    .label(desc.as_str())
                    .xalign(0.0)
                    .css_classes(["dim-label"])
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .build();
                vbox.append(&lbl_desc);
            }
        }
        hbox.append(&vbox);

        // ── Backend rozeti (renkli pill) ─────────────────────────────────────
        let (badge_text, badge_class) = match snap.backend {
            BackendKind::Btrfs   => ("Btrfs", "accent"),
            BackendKind::LvmThin => ("LVM",   "warning"),
            BackendKind::Rsync   => ("Rsync",  "success"),
        };
        let badge = gtk4::Label::builder()
            .label(badge_text)
            .valign(gtk4::Align::Center)
            .build();
        badge.add_css_class("pill");
        badge.add_css_class("caption");
        badge.add_css_class(badge_class);
        hbox.append(&badge);

        // ── Boyut etiketi ────────────────────────────────────────────────────
        let lbl_size = gtk4::Label::builder()
            .label(snap.size_human())
            .css_classes(["dim-label", "numeric"])
            .valign(gtk4::Align::Center)
            .build();
        hbox.append(&lbl_size);

        // ── Kilit ikonu ──────────────────────────────────────────────────────
        if snap.locked {
            let lock_icon = gtk4::Image::from_icon_name("changes-prevent-symbolic");
            lock_icon.set_valign(gtk4::Align::Center);
            lock_icon.set_tooltip_text(Some("Kilitli"));
            hbox.append(&lock_icon);
        }

        // ── ID rozeti ────────────────────────────────────────────────────────
        let lbl_id = gtk4::Label::builder()
            .label(format!("#{}", snap.id))
            .css_classes(["pill", "monospace"])
            .valign(gtk4::Align::Center)
            .build();
        hbox.append(&lbl_id);

        let row = gtk4::ListBoxRow::new();
        row.set_child(Some(&hbox));
        row.set_widget_name(&snap.id.to_string());

        SnapshotRow { row }
    }

    pub fn widget(&self) -> &gtk4::ListBoxRow {
        &self.row
    }
}
