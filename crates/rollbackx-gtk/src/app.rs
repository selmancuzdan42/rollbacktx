//! Ana uygulama penceresi ve UI ağacı.

use gtk4::prelude::*;
use libadwaita::prelude::*;

use crate::dialogs;
use crate::snapshot_row::SnapshotRow;
use crate::tray::TrayCmd;
use rollbackx_core::backend::is_snapshot_mounted;
use rollbackx_core::snapshot::Snapshot;

use rollbackx_core::DB_PATH;

// ── Kullanıcı modu tespiti ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KullaniciMod {
    /// Tek kullanıcılı sistem veya root — kısıtlama yok
    Admin,
    /// 2+ kullanıcı, sudo grubunda — tam yetki
    Ogretmen,
    /// 2+ kullanıcı, sudo grubunda değil — sadece görüntüle + geri yükle
    Ogrenci,
}

fn kullanici_modu() -> KullaniciMod {
    // Gerçek kullanıcı sayısını say (UID 1000–65533)
    let gercek_kullanici_sayisi = std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            let p: Vec<&str> = l.split(':').collect();
            p.get(2)
                .and_then(|uid| uid.parse::<u32>().ok())
                .map(|uid| (1000..65534).contains(&uid))
                .unwrap_or(false)
        })
        .count();

    if gercek_kullanici_sayisi <= 1 {
        return KullaniciMod::Admin;
    }

    if sudo_grubunda_mi() {
        KullaniciMod::Ogretmen
    } else {
        KullaniciMod::Ogrenci
    }
}

fn sudo_grubunda_mi() -> bool {
    let kullanici = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    if kullanici.is_empty() {
        return false;
    }
    let grup_dosyasi = std::fs::read_to_string("/etc/group").unwrap_or_default();
    for satir in grup_dosyasi.lines() {
        let p: Vec<&str> = satir.split(':').collect();
        let grup_adi = match p.first() { Some(g) => *g, None => continue };
        if grup_adi == "sudo" || grup_adi == "admin" || grup_adi == "wheel" {
            if let Some(uyeler) = p.get(3) {
                if uyeler.split(',').any(|u| u.trim() == kullanici) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn build_ui(
    app: &libadwaita::Application,
    arka_plan: bool,
    tray_rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<TrayCmd>>>,
) {
    // ── Kullanıcı modu ────────────────────────────────────────────────────────
    let mod_ = kullanici_modu();
    let yonetici = mod_ != KullaniciMod::Ogrenci;

    // ── Ana pencere ───────────────────────────────────────────────────────────
    let win = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("RollbackX")
        .default_width(750)
        .default_height(520)
        .build();

    // Pencere kapatılınca yok etme, sadece gizle
    win.connect_close_request({
        let win = win.clone();
        move |_| {
            win.set_visible(false);
            glib::Propagation::Stop
        }
    });

    // ── Header bar ────────────────────────────────────────────────────────────
    let header = libadwaita::HeaderBar::new();

    let btn_new = gtk4::Button::from_icon_name("list-add-symbolic");
    btn_new.set_tooltip_text(Some("Yeni Snapshot Oluştur"));
    btn_new.add_css_class("suggested-action");

    let btn_refresh = gtk4::Button::from_icon_name("view-refresh-symbolic");
    btn_refresh.set_tooltip_text(Some("Listeyi Yenile"));

    let btn_schedule = gtk4::Button::from_icon_name("alarm-symbolic");
    btn_schedule.set_tooltip_text(Some("Zamanlanmış Geri Yükleme"));

    let btn_oto = gtk4::Button::from_icon_name("system-reboot-symbolic");
    btn_oto.set_tooltip_text(Some("Önyükleme Otomatik Geri Yükleme"));

    let btn_import = gtk4::Button::from_icon_name("document-save-symbolic");
    btn_import.set_tooltip_text(Some("Snapshot İçe Aktar (.rxsnap)"));

    let btn_about = gtk4::Button::from_icon_name("help-about-symbolic");
    btn_about.set_tooltip_text(Some("Hakkında"));

    let btn_settings = gtk4::Button::from_icon_name("preferences-system-symbolic");
    btn_settings.set_tooltip_text(Some("Ayarlar"));

    // Yönetici değilse tüm işlem butonlarını gizle
    btn_new.set_visible(yonetici);
    btn_import.set_visible(yonetici);
    btn_schedule.set_visible(yonetici);
    btn_oto.set_visible(yonetici);
    btn_settings.set_visible(yonetici);

    header.pack_start(&btn_new);
    header.pack_start(&btn_refresh);
    header.pack_start(&btn_import);
    header.pack_end(&btn_about);
    header.pack_end(&btn_settings);
    header.pack_end(&btn_schedule);
    header.pack_end(&btn_oto);

    // Öğrenci modu badge'i
    if mod_ == KullaniciMod::Ogrenci {
        let badge = gtk4::Label::builder()
            .label("Öğrenci Modu")
            .build();
        badge.add_css_class("pill");
        badge.add_css_class("caption");
        badge.add_css_class("warning");
        header.pack_end(&badge);
    }

    // ── Backend banner ────────────────────────────────────────────────────────
    let backend_banner = libadwaita::Banner::new("Backend tespit ediliyor...");
    backend_banner.set_revealed(true);

    // ── Snapshot listesi ──────────────────────────────────────────────────────
    let list_box = gtk4::ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::Single);
    list_box.add_css_class("boxed-list");
    list_box.set_margin_top(12);
    list_box.set_margin_bottom(12);
    list_box.set_margin_start(12);
    list_box.set_margin_end(12);

    let empty_page = libadwaita::StatusPage::new();
    empty_page.set_title("Snapshot Yok");
    empty_page.set_description(Some(if yonetici {
        "Yeni snapshot oluşturmak için + düğmesine basın."
    } else {
        "Henüz geri yüklenebilir snapshot bulunmuyor. Öğretmeninizden snapshot oluşturmasını isteyin."
    }));
    empty_page.set_icon_name(Some("camera-photo-symbolic"));

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::Crossfade);

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .child(&list_box)
        .build();

    stack.add_named(&scrolled, Some("list"));
    stack.add_named(&empty_page, Some("empty"));

    // ── Alt eylem çubuğu ─────────────────────────────────────────────────────
    let btn_restore = gtk4::Button::new();
    btn_restore.set_label("Geri Yükle");
    btn_restore.set_sensitive(false);

    let btn_delete = gtk4::Button::new();
    btn_delete.set_label("Sil");
    btn_delete.add_css_class("destructive-action");
    btn_delete.set_sensitive(false);

    let btn_info = gtk4::Button::new();
    btn_info.set_label("Detay");
    btn_info.set_sensitive(false);

    let btn_ac = gtk4::Button::new();
    btn_ac.set_label("Dosyalarda Aç");
    btn_ac.set_sensitive(false);
    btn_ac.set_tooltip_text(Some("Snapshot'ı dosya yöneticisinde aç"));

    let btn_kilitle = gtk4::Button::new();
    btn_kilitle.set_label("Kilitle");
    btn_kilitle.set_sensitive(false);
    btn_kilitle.set_tooltip_text(Some("Snapshot'ı kilitle / kilidini kaldır"));

    let btn_disaaktar = gtk4::Button::new();
    btn_disaaktar.set_label("Dışa Aktar");
    btn_disaaktar.set_sensitive(false);
    btn_disaaktar.set_tooltip_text(Some("Snapshot'ı .rxsnap dosyasına aktar"));

    // ── Overflow menü (⋮) — Düzenle + Doğrula ────────────────────────────────
    let pop_duzenle = gtk4::Button::builder().label("Düzenle").halign(gtk4::Align::Fill).build();
    let pop_dogrula = gtk4::Button::builder().label("Doğrula").halign(gtk4::Align::Fill).build();

    for b in [&pop_duzenle, &pop_dogrula] {
        b.add_css_class("flat");
    }

    let pop_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    pop_box.set_margin_top(4);
    pop_box.set_margin_bottom(4);
    pop_box.set_margin_start(4);
    pop_box.set_margin_end(4);
    pop_box.append(&pop_duzenle);
    pop_box.append(&pop_dogrula);

    let popover = gtk4::Popover::new();
    popover.set_child(Some(&pop_box));

    let btn_menu = gtk4::MenuButton::new();
    btn_menu.set_icon_name("view-more-symbolic");
    btn_menu.set_tooltip_text(Some("Daha fazla seçenek"));
    btn_menu.set_popover(Some(&popover));
    btn_menu.set_sensitive(false);
    btn_menu.set_visible(yonetici);

    btn_restore.set_visible(yonetici);
    btn_ac.set_visible(yonetici);
    btn_delete.set_visible(yonetici);
    btn_kilitle.set_visible(yonetici);
    btn_disaaktar.set_visible(yonetici);

    let action_bar = gtk4::ActionBar::new();
    action_bar.pack_start(&btn_restore);
    action_bar.pack_start(&btn_ac);
    action_bar.pack_end(&btn_delete);
    action_bar.pack_end(&btn_kilitle);
    action_bar.pack_end(&btn_disaaktar);
    action_bar.pack_end(&btn_menu);
    action_bar.pack_end(&btn_info);

    // ── Disk kullanım çubuğu ─────────────────────────────────────────────────
    let usage_bar = gtk4::LevelBar::new();
    usage_bar.set_min_value(0.0);
    usage_bar.set_max_value(1.0);
    usage_bar.set_margin_start(12);
    usage_bar.set_margin_end(12);
    usage_bar.set_margin_top(6);
    usage_bar.set_margin_bottom(2);

    let usage_label = gtk4::Label::builder()
        .xalign(1.0)
        .css_classes(["caption", "dim-label"])
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(6)
        .build();

    // ── Layout ────────────────────────────────────────────────────────────────
    let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content_box.append(&backend_banner);
    content_box.append(&usage_bar);
    content_box.append(&usage_label);
    content_box.append(&stack);
    content_box.append(&action_bar);

    // ToastOverlay — başarı bildirimleri için
    let toast_overlay = libadwaita::ToastOverlay::new();
    toast_overlay.set_child(Some(&content_box));
    dialogs::set_toast_overlay(toast_overlay.clone());

    let toolbar_view = libadwaita::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&toast_overlay));

    win.set_content(Some(&toolbar_view));

    // ── Seçili snapshot ID ────────────────────────────────────────────────────
    let selected_id = std::rc::Rc::new(std::cell::Cell::new(None::<u32>));

    // ── Liste yükleme ─────────────────────────────────────────────────────────
    let load_fn = {
        let list_box       = list_box.clone();
        let stack          = stack.clone();
        let backend_banner = backend_banner.clone();
        let usage_bar      = usage_bar.clone();
        let usage_label    = usage_label.clone();
        let btn_restore    = btn_restore.clone();
        let btn_delete     = btn_delete.clone();
        let btn_info       = btn_info.clone();
        let btn_ac         = btn_ac.clone();
        let btn_kilitle    = btn_kilitle.clone();
        let btn_disaaktar  = btn_disaaktar.clone();
        let btn_menu       = btn_menu.clone();
        let selected_id    = selected_id.clone();

        std::rc::Rc::new(move || {
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }
            selected_id.set(None);
            btn_restore.set_sensitive(false);
            btn_delete.set_sensitive(false);
            btn_info.set_sensitive(false);
            btn_ac.set_label("Dosyalarda Aç");
            btn_ac.set_sensitive(false);
            btn_kilitle.set_label("Kilitle");
            btn_kilitle.set_sensitive(false);
            btn_disaaktar.set_sensitive(false);
            btn_menu.set_sensitive(false);
            backend_banner.set_title(&detect_backend_label());

            match load_snapshots_from_db() {
                Ok(snaps) if snaps.is_empty() => {
                    stack.set_visible_child_name("empty");
                    usage_bar.set_value(0.0);
                    usage_label.set_label("Snapshot yok");
                }
                Ok(snaps) => {
                    stack.set_visible_child_name("list");
                    for snap in &snaps {
                        list_box.append(SnapshotRow::new(snap).widget());
                    }
                    // Disk kullanım çubuğunu güncelle
                    let total: u64 = snaps.iter().filter_map(|s| s.size_bytes).sum();
                    let avail = disk_available_bytes("/var/lib/rollbackx/snapshots")
                        .unwrap_or(0);
                    let fraction = if avail + total > 0 {
                        (total as f64) / ((total + avail) as f64)
                    } else { 0.0 };
                    usage_bar.set_value(fraction.min(1.0));
                    usage_label.set_label(&format!(
                        "{} snapshot kullanımı · {} boş",
                        format_bytes(total), format_bytes(avail)
                    ));
                }
                Err(e) => {
                    backend_banner.set_title(&format!("DB okunamadı: {e}"));
                    stack.set_visible_child_name("empty");
                }
            }
        })
    };

    // ── Seçim ────────────────────────────────────────────────────────────────
    list_box.connect_row_selected({
        let btn_restore = btn_restore.clone();
        let btn_delete  = btn_delete.clone();
        let btn_info    = btn_info.clone();
        let btn_ac      = btn_ac.clone();
        let btn_kilitle  = btn_kilitle.clone();
        let btn_disaaktar = btn_disaaktar.clone();
        let btn_menu     = btn_menu.clone();
        let selected_id  = selected_id.clone();
        move |_, row| {
            let has = row.is_some();
            btn_restore.set_sensitive(has);
            btn_delete.set_sensitive(has);
            btn_info.set_sensitive(has);
            btn_ac.set_sensitive(has);
            btn_kilitle.set_sensitive(has);
            btn_disaaktar.set_sensitive(has);
            btn_menu.set_sensitive(has);
            if let Some(r) = row {
                if let Ok(id) = r.widget_name().to_string().parse::<u32>() {
                    selected_id.set(Some(id));
                    // Mount durumu → btn_ac etiketi
                    let mp = rollbackx_core::backend::snapshot_mount_point(id);
                    if is_snapshot_mounted(&mp) {
                        btn_ac.set_label("Çıkar");
                    } else {
                        btn_ac.set_label("Dosyalarda Aç");
                    }
                    // Kilit durumu → btn_kilitle etiketi
                    if let Ok(snaps) = load_snapshots_from_db() {
                        if let Some(snap) = snaps.iter().find(|s| s.id == id) {
                            btn_kilitle.set_label(if snap.locked { "Kilidi Kaldır" } else { "Kilitle" });
                        }
                    }
                }
            } else {
                selected_id.set(None);
                btn_ac.set_label("Dosyalarda Aç");
                btn_kilitle.set_label("Kilitle");
            }
        }
    });

    // ── Yeni snapshot ─────────────────────────────────────────────────────────
    let show_create = {
        let win     = win.clone();
        let load_fn = load_fn.clone();
        move || {
            let w = win.clone().upcast::<gtk4::Window>();
            let l = load_fn.clone();
            dialogs::show_create_dialog(w, move || l());
        }
    };

    btn_new.connect_clicked({
        let sc = show_create.clone();
        move |_| sc()
    });

    // ── Yenile ────────────────────────────────────────────────────────────────
    btn_refresh.connect_clicked({
        let load_fn = load_fn.clone();
        move |_| load_fn()
    });

    // ── Geri yükle ───────────────────────────────────────────────────────────
    btn_restore.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                let w = win.clone().upcast::<gtk4::Window>();
                let l = load_fn.clone();
                dialogs::show_restore_dialog(w, id, move || l());
            }
        }
    });

    // ── Sil ──────────────────────────────────────────────────────────────────
    btn_delete.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                let w = win.clone().upcast::<gtk4::Window>();
                let l = load_fn.clone();
                dialogs::show_delete_dialog(w, id, move || l());
            }
        }
    });

    // ── Detay ────────────────────────────────────────────────────────────────
    btn_info.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                if let Ok(snaps) = load_snapshots_from_db() {
                    if let Some(snap) = snaps.iter().find(|s| s.id == id) {
                        dialogs::show_info_dialog(win.upcast_ref::<gtk4::Window>(), snap);
                    }
                }
            }
        }
    });

    // ── Dosyalarda Aç / Çıkar ────────────────────────────────────────────────
    btn_ac.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        let btn_ac      = btn_ac.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                let mp = rollbackx_core::backend::snapshot_mount_point(id);
                if is_snapshot_mounted(&mp) {
                    // Ayır
                    let w = win.clone().upcast::<gtk4::Window>();
                    let l = load_fn.clone();
                    let b = btn_ac.clone();
                    dialogs::show_unmount_dialog(w, id, move || {
                        l();
                        b.set_label("Dosyalarda Aç");
                    });
                } else {
                    // Bağla ve aç
                    let w = win.clone().upcast::<gtk4::Window>();
                    let l = load_fn.clone();
                    let b = btn_ac.clone();
                    dialogs::show_mount_open_dialog(w, id, move || {
                        l();
                        b.set_label("Çıkar");
                    });
                }
            }
        }
    });

    // ── Overflow menü handler'ları ───────────────────────────────────────────

    // Düzenle
    pop_duzenle.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        let popover     = popover.clone();
        move |_| {
            popover.popdown();
            if let Some(id) = selected_id.get() {
                if let Ok(snaps) = load_snapshots_from_db() {
                    if let Some(snap) = snaps.iter().find(|s| s.id == id) {
                        let w = win.clone().upcast::<gtk4::Window>();
                        let l = load_fn.clone();
                        dialogs::show_rename_dialog(w, snap, move || l());
                    }
                }
            }
        }
    });

    // ── Dışa Aktar ───────────────────────────────────────────────────────────
    btn_disaaktar.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                let w = win.clone().upcast::<gtk4::Window>();
                let l = load_fn.clone();
                dialogs::show_export_dialog(w, id, move || l());
            }
        }
    });

    // ── Kilitle / Kilidi Kaldır ───────────────────────────────────────────────
    btn_kilitle.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let load_fn     = load_fn.clone();
        move |_| {
            if let Some(id) = selected_id.get() {
                let locked = load_snapshots_from_db()
                    .unwrap_or_default()
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.locked)
                    .unwrap_or(false);
                let w = win.clone().upcast::<gtk4::Window>();
                let l = load_fn.clone();
                dialogs::show_lock_dialog(w, id, locked, move || l());
            }
        }
    });

    // Doğrula
    pop_dogrula.connect_clicked({
        let win         = win.clone();
        let selected_id = selected_id.clone();
        let popover     = popover.clone();
        move |_| {
            popover.popdown();
            if let Some(id) = selected_id.get() {
                let w = win.clone().upcast::<gtk4::Window>();
                dialogs::show_verify_dialog(w, id);
            }
        }
    });

    // ── Hakkında ─────────────────────────────────────────────────────────────
    btn_about.connect_clicked({
        let win = win.clone();
        move |_| {
            let about = libadwaita::AboutDialog::new();
            about.set_application_name("RollbackX");
            about.set_version(env!("CARGO_PKG_VERSION"));
            about.set_developer_name("RollbackX Contributors");
            about.set_license_type(gtk4::License::Gpl30);
            about.set_comments(
                "Pardus Linux için Btrfs, LVM Thin ve Rsync tabanlı\n\
                 sistem anlık görüntüsü yöneticisi.",
            );
            about.present(Some(&win));
        }
    });

    // ── İçe Aktar ────────────────────────────────────────────────────────────
    btn_import.connect_clicked({
        let win     = win.clone();
        let load_fn = load_fn.clone();
        move |_| {
            let w = win.clone().upcast::<gtk4::Window>();
            let l = load_fn.clone();
            dialogs::show_import_dialog(w, move || l());
        }
    });

    // ── Ayarlar ──────────────────────────────────────────────────────────────
    btn_settings.connect_clicked({
        let win = win.clone();
        move |_| {
            let w = win.clone().upcast::<gtk4::Window>();
            dialogs::show_settings_dialog(w);
        }
    });

    // ── Zamanlayıcı ──────────────────────────────────────────────────────────
    btn_schedule.connect_clicked({
        let win     = win.clone();
        let load_fn = load_fn.clone();
        move |_| {
            let snaps: Vec<(u32, String)> = load_snapshots_from_db()
                .unwrap_or_default()
                .into_iter()
                .map(|s| (s.id, s.name.clone()))
                .collect();
            let w = win.clone().upcast::<gtk4::Window>();
            let l = load_fn.clone();
            dialogs::show_schedule_dialog(w, snaps, move || l());
        }
    });

    // ── Önyükleme Otomatik Geri Yükleme ─────────────────────────────────────
    btn_oto.connect_clicked({
        let win     = win.clone();
        let load_fn = load_fn.clone();
        move |_| {
            let w = win.clone().upcast::<gtk4::Window>();
            let l = load_fn.clone();
            dialogs::show_auto_restore_dialog(w, move || l());
        }
    });

    // ── Klavye kısayolları ─────────────────────────────────────────────────────
    {
        let ctrl = gtk4::EventControllerKey::new();

        let show_create_k = show_create.clone();
        let load_fn_k     = load_fn.clone();
        let win_k         = win.clone();
        let selected_id_k = selected_id.clone();
        let load_fn_k2    = load_fn.clone();
        let load_fn_k3    = load_fn.clone();

        ctrl.connect_key_pressed(move |_, key, _, mods| {
            let ctrl_held = mods.contains(gtk4::gdk::ModifierType::CONTROL_MASK);
            match (ctrl_held, key) {
                // Ctrl+N → Yeni snapshot
                (true, gtk4::gdk::Key::n) => {
                    show_create_k();
                    glib::Propagation::Stop
                }
                // Ctrl+R → Yenile
                (true, gtk4::gdk::Key::r) => {
                    load_fn_k();
                    glib::Propagation::Stop
                }
                // Delete → Seçili snapshot'ı sil
                (false, gtk4::gdk::Key::Delete) => {
                    if let Some(id) = selected_id_k.get() {
                        let w = win_k.clone().upcast::<gtk4::Window>();
                        let l = load_fn_k2.clone();
                        dialogs::show_delete_dialog(w, id, move || l());
                    }
                    glib::Propagation::Stop
                }
                // Ctrl+E → Dışa aktar
                (true, gtk4::gdk::Key::e) => {
                    if let Some(id) = selected_id_k.get() {
                        let w = win_k.clone().upcast::<gtk4::Window>();
                        let l = load_fn_k3.clone();
                        dialogs::show_export_dialog(w, id, move || l());
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });

        win.add_controller(ctrl);
    }

    // ── Tray komutlarını dinle (glib timer ile polling) ───────────────────────
    {
        let win        = win.clone();
        let show_create = show_create.clone();
        let app        = app.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            // Non-blocking: beklemeden mevcut komutu al
            match tray_rx.lock().unwrap().try_recv() {
                Ok(TrayCmd::Show) => {
                    win.present();
                }
                Ok(TrayCmd::NewSnapshot) => {
                    win.present();
                    show_create();
                }
                Ok(TrayCmd::Quit) => {
                    app.quit();
                }
                Err(_) => {}
            }
            glib::ControlFlow::Continue
        });
    }

    // ── Pencere görünür hale gelince listeyi yenile ───────────────────────────
    // connect_map: pencere her ekrana çıktığında tetiklenir (gizle→göster dahil)
    win.connect_map({
        let load_fn = load_fn.clone();
        move |_| { load_fn(); }
    });

    // ── İlk yükleme ve pencere gösterimi ─────────────────────────────────────
    load_fn();

    if arka_plan {
        // Arka plan: pencereyi gösterme, app'i diri tut
        std::mem::forget(app.hold());
        let _ = std::process::Command::new("notify-send")
            .args([
                "--icon=camera-photo-symbolic",
                "RollbackX",
                "Arka planda çalışıyor.\nSistem tepsisinden yönetebilirsiniz.",
            ])
            .spawn();
    } else {
        win.present();
    }

    // İkinci instance gelince pencereyi göster
    app.connect_activate(move |_| {
        win.present();
    });
}

fn load_snapshots_from_db() -> anyhow::Result<Vec<Snapshot>> {
    let content = std::fs::read_to_string(DB_PATH)?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&content)?)
}

fn detect_backend_label() -> String {
    let btrfs = std::fs::read_to_string("/proc/self/mountinfo")
        .map(|s| {
            s.lines().any(|l| {
                let p: Vec<&str> = l.split_whitespace().collect();
                p.get(4) == Some(&"/")
                    && p.windows(2).any(|w| w[0] == "-" && w[1] == "btrfs")
            })
        })
        .unwrap_or(false);

    if btrfs {
        return "Backend: Btrfs".to_string();
    }

    let lvm = std::process::Command::new("lvs")
        .args(["--noheadings", "-o", "lv_attr"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains('V'))
        .unwrap_or(false);

    if lvm {
        "Backend: LVM Thin".to_string()
    } else {
        "Backend: Rsync (ext4 / evrensel)".to_string()
    }
}

fn disk_available_bytes(path: &str) -> Option<u64> {
    let out = std::process::Command::new("df")
        .args(["-B1", "--output=avail", path])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().nth(1)?.trim().parse().ok()
}

fn format_bytes(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.0} MB", b as f64 / (1u64 << 20) as f64)
    } else {
        format!("{} KB", b / 1024)
    }
}
