//! Dialog pencereler: snapshot oluştur, geri yükle, sil, detay.
//!
//! Tüm yazma işlemleri `pkexec rollbackx <komut>` ile root yetkisiyle çalışır.
//! pkexec çağrıları ayrı thread'de çalışır; beklenirken spinner gösterilir.

use std::rc::Rc;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::io::{BufRead, BufReader, Read};
use std::process::Stdio;

use gtk4::prelude::*;
use libadwaita::prelude::*;

use rollbackx_core::snapshot::{Snapshot, BackendKind};
use rollbackx_core::schedule::Schedule;

use rollbackx_core::DB_PATH;

/// Callback tipi — dialog'larda tek seferlik çağrı için `Rc<RefCell<Option<...>>>` pattern.
type OnDoneCell = Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>;
type OnDoneRcCell = Rc<RefCell<Option<Rc<dyn Fn() + 'static>>>>;

// ── Toast overlay (thread-local) ──────────────────────────────────────────────

thread_local! {
    static TOAST_OVERLAY: RefCell<Option<libadwaita::ToastOverlay>> = const { RefCell::new(None) };
}

pub fn set_toast_overlay(overlay: libadwaita::ToastOverlay) {
    TOAST_OVERLAY.with(|tl| *tl.borrow_mut() = Some(overlay));
}

pub(crate) fn show_toast(msg: &str) {
    let msg = msg.to_string();
    TOAST_OVERLAY.with(|tl| {
        if let Some(overlay) = tl.borrow().as_ref() {
            let toast = libadwaita::Toast::new(&msg);
            toast.set_timeout(3);
            overlay.add_toast(toast);
        }
    });
}

fn load_snapshots() -> anyhow::Result<Vec<Snapshot>> {
    let content = std::fs::read_to_string(DB_PATH)?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_str(&content)?)
}

// ── pkexec (thread'e gönderilebilir) ────────────────────────────────────────

fn pkexec_rollbackx_owned(args: Vec<String>) -> Result<String, String> {
    let mut cmd = std::process::Command::new("pkexec");
    cmd.arg("rollbackx");
    for a in &args { cmd.arg(a); }
    match cmd.output() {
        Err(e)  => Err(format!("pkexec başlatılamadı: {e}")),
        Ok(out) => {
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).to_string())
            } else {
                let s = String::from_utf8_lossy(&out.stderr).trim().to_string();
                Err(if s.is_empty() { "Bilinmeyen hata".into() } else { s })
            }
        }
    }
}

// ── Sonuç dialog'u ───────────────────────────────────────────────────────────

fn show_result_dialog(parent: &gtk4::Window, baslik: &str, msg: &str, hata_mi: bool) {
    let dialog = libadwaita::AlertDialog::new(Some(baslik), Some(msg));
    dialog.add_response("ok", "Tamam");
    dialog.set_default_response(Some("ok"));
    if hata_mi {
        dialog.set_response_appearance("ok", libadwaita::ResponseAppearance::Destructive);
    }
    dialog.present(Some(parent));
}

// ── Spinner dialog'u ─────────────────────────────────────────────────────────

fn show_progress_dialog(parent: &gtk4::Window, mesaj: &str) -> libadwaita::Dialog {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title(mesaj);
    dialog.set_content_width(280);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 20);
    vbox.set_margin_top(40);
    vbox.set_margin_bottom(40);
    vbox.set_margin_start(32);
    vbox.set_margin_end(32);
    vbox.set_halign(gtk4::Align::Center);

    let spinner = gtk4::Spinner::new();
    spinner.set_size_request(48, 48);
    spinner.set_halign(gtk4::Align::Center);
    spinner.start();

    let label = gtk4::Label::new(Some(mesaj));
    label.add_css_class("dim-label");
    label.set_wrap(true);
    label.set_max_width_chars(28);
    label.set_halign(gtk4::Align::Center);

    vbox.append(&spinner);
    vbox.append(&label);
    dialog.set_child(Some(&vbox));
    dialog.present(Some(parent));
    dialog
}

// ── Arka plan çalıştırıcı ─────────────────────────────────────────────────────
//
// pkexec'i ayrı thread'de çalıştırır, spinner gösterir.
// 50 ms'de bir poll ederek sonuç gelince callback'leri tetikler.

fn run_with_progress(
    parent:    gtk4::Window,
    mesaj:     &'static str,
    args:      Vec<String>,
    on_done:   impl Fn() + 'static,
    on_result: impl Fn(Result<String, String>, &gtk4::Window) + 'static,
) {
    let progress   = show_progress_dialog(&parent, mesaj);
    let progress_c = progress.clone();

    // Thread'den ana thread'e sonuç taşıyacak shared slot
    let slot: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot_thread = std::sync::Arc::clone(&slot);

    std::thread::spawn(move || {
        let result = pkexec_rollbackx_owned(args);
        *slot_thread.lock().unwrap() = Some(result);
    });

    // 50 ms'de bir kontrol et; sonuç gelince spinner'ı kapat
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let mut guard = slot.lock().unwrap();
        if guard.is_none() {
            return glib::ControlFlow::Continue;
        }
        let result = guard.take().unwrap();
        drop(guard);
        progress_c.close();
        on_done();
        on_result(result, &parent);
        glib::ControlFlow::Break
    });
}

// ── Yüzdelik progress bar dialog + streaming runner ──────────────────────────
//
// Snapshot oluşturma ve içe aktarma gibi rsync tabanlı işlemler için.
// pkexec çıktısından "PROGRESS:N" satırları okunarak bar güncellenir.

fn show_progress_bar_dialog(
    parent: &gtk4::Window,
    mesaj:  &str,
) -> (libadwaita::Dialog, gtk4::ProgressBar, gtk4::Label) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title(mesaj);
    dialog.set_content_width(360);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(32);
    vbox.set_margin_bottom(32);
    vbox.set_margin_start(32);
    vbox.set_margin_end(32);

    let lbl = gtk4::Label::new(Some(mesaj));
    lbl.add_css_class("dim-label");
    lbl.set_wrap(true);
    lbl.set_halign(gtk4::Align::Center);

    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_fraction(0.0);
    progress_bar.set_show_text(false);

    let status_lbl = gtk4::Label::builder()
        .label("Hazırlanıyor…")
        .css_classes(["caption", "dim-label"])
        .halign(gtk4::Align::Center)
        .build();

    vbox.append(&lbl);
    vbox.append(&progress_bar);
    vbox.append(&status_lbl);
    dialog.set_child(Some(&vbox));
    dialog.present(Some(parent));

    (dialog, progress_bar, status_lbl)
}

fn run_with_progress_bar(
    parent:    gtk4::Window,
    mesaj:     &'static str,
    args:      Vec<String>,
    on_done:   impl Fn() + 'static,
    on_result: impl Fn(Result<String, String>, &gtk4::Window) + 'static,
) {
    let (dialog, progress_bar, status_lbl) = show_progress_bar_dialog(&parent, mesaj);
    let dialog_c = dialog.clone();

    // Paylaşımlı durum: yüzde + sonuç
    let pct_slot:    Arc<Mutex<f64>>                      = Arc::new(Mutex::new(0.0));
    let result_slot: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
    let pct_t    = Arc::clone(&pct_slot);
    let result_t = Arc::clone(&result_slot);

    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("pkexec");
        cmd.arg("rollbackx");
        for a in &args { cmd.arg(a); }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c)  => c,
            Err(e) => {
                *result_t.lock().unwrap() =
                    Some(Err(format!("pkexec başlatılamadı: {e}")));
                return;
            }
        };

        let stderr  = child.stderr.take().unwrap();
        let stderr_thread = std::thread::spawn(move || {
            let mut s = String::new();
            BufReader::new(stderr).read_to_string(&mut s).ok();
            s
        });

        let stdout = child.stdout.take().unwrap();
        for line in BufReader::new(stdout).lines().map_while(|r| r.ok()) {
            if let Some(rest) = line.strip_prefix("PROGRESS:") {
                if let Ok(p) = rest.trim().parse::<u8>() {
                    *pct_t.lock().unwrap() = p as f64 / 100.0;
                }
            }
        }

        let status          = child.wait();
        let stderr_content  = stderr_thread.join().unwrap_or_default();

        let result = match status {
            Err(e) => Err(format!("Bekleme hatası: {e}")),
            Ok(s) if s.success() => Ok(String::new()),
            Ok(s) => {
                let code = s.code().unwrap_or(-1);
                if code == 126 {
                    Err("Yetki reddedildi".to_string())
                } else if stderr_content.trim().is_empty() {
                    Err("Bilinmeyen hata".to_string())
                } else {
                    Err(stderr_content.trim().to_string())
                }
            }
        };
        *result_t.lock().unwrap() = Some(result);
    });

    // 50 ms'de bir bar'ı güncelle; tamamlanınca kapat
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let pct = *pct_slot.lock().unwrap();
        progress_bar.set_fraction(pct);
        if pct > 0.0 {
            status_lbl.set_text(&format!("%{:.0} tamamlandı", pct * 100.0));
        }

        let mut guard = result_slot.lock().unwrap();
        if guard.is_none() {
            return glib::ControlFlow::Continue;
        }
        let result = guard.take().unwrap();
        drop(guard);
        dialog_c.close();
        on_done();
        on_result(result, &parent);
        glib::ControlFlow::Break
    });
}

// ── Snapshot Oluştur ─────────────────────────────────────────────────────────

pub fn show_create_dialog(parent: gtk4::Window, on_done: impl Fn() + 'static) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Yeni Snapshot Oluştur");
    dialog.set_content_width(400);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let isim_row    = libadwaita::EntryRow::new();
    isim_row.set_title("Snapshot İsmi");

    let aciklama_row = libadwaita::EntryRow::new();
    aciklama_row.set_title("Açıklama (opsiyonel)");

    let group = libadwaita::PreferencesGroup::new();
    group.add(&isim_row);
    group.add(&aciklama_row);
    vbox.append(&group);

    let warn = gtk4::Label::new(Some("⚠ Bu işlem yönetici yetkisi gerektirir."));
    warn.add_css_class("dim-label");
    warn.add_css_class("caption");
    warn.set_halign(gtk4::Align::Start);
    vbox.append(&warn);

    let btn_box     = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let btn_iptal   = gtk4::Button::with_label("İptal");
    let btn_olustur = gtk4::Button::with_label("Oluştur");
    btn_olustur.add_css_class("suggested-action");
    btn_box.append(&btn_iptal);
    btn_box.append(&btn_olustur);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));

    btn_iptal.connect_clicked({
        let dialog = dialog.clone();
        move |_| { dialog.close(); }
    });

    // on_done'ı sadece bir kez kullanılabilir hale getir (Rc<RefCell<Option<...>>>)
    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));

    btn_olustur.connect_clicked({
        let dialog       = dialog.clone();
        let isim_row     = isim_row.clone();
        let aciklama_row = aciklama_row.clone();
        let parent       = parent.clone();
        let on_done_cell = on_done_cell.clone();

        move |_| {
            let isim = isim_row.text().to_string();
            if isim.trim().is_empty() {
                isim_row.add_css_class("error");
                return;
            }
            isim_row.remove_css_class("error");

            let aciklama = aciklama_row.text().to_string();
            dialog.close();

            let mut args = vec!["snapshot".to_string(), "create".to_string(), isim.clone()];
            if !aciklama.is_empty() {
                args.push("--açıklama".to_string());
                args.push(aciklama);
            }

            let taken = on_done_cell.borrow_mut().take();
            let isim2 = isim.clone();
            run_with_progress_bar(
                parent.clone(),
                "Snapshot oluşturuluyor...",
                args,
                move || { if let Some(f) = &taken { f(); } },
                move |result, win| match result {
                    Ok(_) => show_toast(&format!("\"{}\" başarıyla oluşturuldu.", isim2)),
                    Err(e) => show_result_dialog(
                        win, "Hata",
                        &format!("Snapshot oluşturulamadı:\n{e}"), true,
                    ),
                },
            );
        }
    });

    dialog.present(Some(&parent));
}

// ── Mount / Browse ────────────────────────────────────────────────────────────

/// Snapshot'ı bağla ve dosya yöneticisinde aç.
pub fn show_mount_open_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    let args = vec![
        "snapshot".to_string(), "bagla".to_string(), id.to_string(),
    ];
    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let taken = on_done_cell.borrow_mut().take();
    run_with_progress(
        parent.clone(),
        "Snapshot bağlanıyor...",
        args,
        move || { if let Some(f) = &taken { f(); } },
        move |result, win| match result {
            Ok(_) => {
                // xdg-open çağrısı (normal kullanıcı, pkexec yok)
                let _ = std::process::Command::new("xdg-open")
                    .arg(format!("/mnt/rollbackx/{id}/"))
                    .spawn();
            }
            Err(e) => show_result_dialog(win, "Hata", &format!("Snapshot bağlanamadı:\n{e}"), true),
        },
    );
}

/// Bağlı snapshot'ı ayır.
pub fn show_unmount_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    let args = vec![
        "snapshot".to_string(), "ayir".to_string(), id.to_string(),
    ];
    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let taken = on_done_cell.borrow_mut().take();
    run_with_progress(
        parent.clone(),
        "Snapshot ayrılıyor...",
        args,
        move || { if let Some(f) = &taken { f(); } },
        move |result, win| match result {
            Ok(_) => {}
            Err(e) => show_result_dialog(win, "Hata", &format!("Snapshot ayrılamadı:\n{e}"), true),
        },
    );
}

// ── Kilitle / Kilit Aç ───────────────────────────────────────────────────────

/// Snapshot kilit durumunu değiştir.
pub fn show_lock_dialog(parent: gtk4::Window, id: u32, currently_locked: bool, on_done: impl Fn() + 'static) {
    let (baslik, aciklama, yanit_etiket, yanit_key) = if currently_locked {
        (
            "Kilidi Kaldır",
            format!("Snapshot #{id} kilidi kaldırılsın mı? Ardından silinebilir ve geri yüklenebilir."),
            "Kilidi Kaldır",
            "kilit_ac",
        )
    } else {
        (
            "Kilitle",
            format!("Snapshot #{id} kilitlensin mi? Kilitli snapshot silinemez ve geri yüklenemez."),
            "Kilitle",
            "kilitle",
        )
    };

    let dialog = libadwaita::AlertDialog::new(Some(baslik), Some(&aciklama));
    dialog.add_response("iptal", "İptal");
    dialog.add_response(yanit_key, yanit_etiket);
    dialog.set_response_appearance(
        yanit_key,
        if currently_locked {
            libadwaita::ResponseAppearance::Default
        } else {
            libadwaita::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some("iptal"));

    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let parent_c = parent.clone();
    let cmd = if currently_locked { "kilit-ac" } else { "kilitle" };

    dialog.connect_response(None, move |_, response| {
        if response == "iptal" { return; }

        let args = vec![
            "snapshot".to_string(), cmd.to_string(), id.to_string(),
        ];
        let taken = on_done_cell.borrow_mut().take();
        let mesaj: &'static str = if currently_locked { "Kilit kaldırılıyor..." } else { "Kilitleniyor..." };
        run_with_progress(
            parent_c.clone(),
            mesaj,
            args,
            move || { if let Some(f) = &taken { f(); } },
            move |result, win| match result {
                Ok(_) => {}
                Err(e) => show_result_dialog(win, "Hata", &format!("Kilit işlemi başarısız:\n{e}"), true),
            },
        );
    });

    dialog.present(Some(&parent));
}

// ── Geri Yükle ───────────────────────────────────────────────────────────────

pub fn show_restore_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    // Kilit kontrolü
    if let Ok(snaps) = load_snapshots() {
        if let Some(snap) = snaps.iter().find(|s| s.id == id) {
            if snap.locked {
                show_result_dialog(
                    &parent,
                    "Kilitli Snapshot",
                    &format!("Snapshot #{id} kilitli — geri yükleme yapılamaz.\nÖnce kilidi kaldırın."),
                    true,
                );
                return;
            }
        }
    }

    let dialog = libadwaita::AlertDialog::new(
        Some("Geri Yükleme Onayı"),
        Some(&format!(
            "Snapshot #{id} geri yüklensin mi?\n\n\
             Bu işlem sisteminizi yeniden başlatacak ve seçili \
             anlık görüntüye dönecektir."
        )),
    );
    dialog.add_response("iptal", "İptal");
    dialog.add_response("geri_yukle", "Geri Yükle ve Yeniden Başlat");
    dialog.set_response_appearance("geri_yukle", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("iptal"));

    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let parent_c = parent.clone();

    dialog.connect_response(None, move |_, response| {
        if response != "geri_yukle" { return; }

        let args = vec![
            "snapshot".to_string(), "restore".to_string(),
            id.to_string(), "--zorla".to_string(),
        ];
        let taken = on_done_cell.borrow_mut().take();
        run_with_progress(
            parent_c.clone(),
            "Geri yükleme hazırlanıyor...",
            args,
            move || { if let Some(f) = &taken { f(); } },
            move |result, win| match result {
                Ok(_) => show_result_dialog(
                    win, "Geri Yükleme Hazır",
                    "Sistem şimdi yeniden başlatılıyor...", false,
                ),
                Err(e) => show_result_dialog(
                    win, "Hata",
                    &format!("Geri yükleme başarısız:\n{e}"), true,
                ),
            },
        );
    });

    dialog.present(Some(&parent));
}

// ── Sil ──────────────────────────────────────────────────────────────────────

pub fn show_delete_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    // Kilit kontrolü
    if let Ok(snaps) = load_snapshots() {
        if let Some(snap) = snaps.iter().find(|s| s.id == id) {
            if snap.locked {
                show_result_dialog(
                    &parent,
                    "Kilitli Snapshot",
                    &format!("Snapshot #{id} kilitli — silinemez.\nÖnce kilidi kaldırın."),
                    true,
                );
                return;
            }
        }
    }

    let dialog = libadwaita::AlertDialog::new(
        Some("Snapshot'ı Sil"),
        Some(&format!(
            "Snapshot #{id} kalıcı olarak silinsin mi?\nBu işlem geri alınamaz."
        )),
    );
    dialog.add_response("iptal", "İptal");
    dialog.add_response("sil", "Kalıcı Olarak Sil");
    dialog.set_response_appearance("sil", libadwaita::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("iptal"));

    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));
    let parent_c = parent.clone();

    dialog.connect_response(None, move |_, response| {
        if response != "sil" { return; }

        let args = vec![
            "snapshot".to_string(), "delete".to_string(),
            id.to_string(), "--zorla".to_string(),
        ];
        let taken = on_done_cell.borrow_mut().take();
        run_with_progress(
            parent_c.clone(),
            "Snapshot siliniyor...",
            args,
            move || { if let Some(f) = &taken { f(); } },
            move |result, win| match result {
                Ok(_) => show_toast(&format!("Snapshot #{id} başarıyla silindi.")),
                Err(e) => show_result_dialog(
                    win, "Hata",
                    &format!("Silme işlemi başarısız:\n{e}"), true,
                ),
            },
        );
    });

    dialog.present(Some(&parent));
}

// ── Detay ─────────────────────────────────────────────────────────────────────

pub fn show_info_dialog(parent: &gtk4::Window, snap: &Snapshot) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title(&format!("Snapshot #{} Detayları", snap.id));
    dialog.set_content_width(420);

    let group = libadwaita::PreferencesGroup::new();
    let satirlar: &[(&str, String)] = &[
        ("İsim",        snap.name.clone()),
        ("Açıklama",    snap.description.clone().unwrap_or_else(|| "—".into())),
        ("Oluşturuldu", snap.created_at.format("%Y-%m-%d %H:%M:%S").to_string()),
        ("Tetikleyici", snap.trigger.to_string()),
        ("Backend",     snap.backend.to_string()),
        ("Referans",    snap.backend_ref.clone()),
        ("Boyut",       snap.size_human()),
    ];
    for (etiket, deger) in satirlar {
        let row = libadwaita::ActionRow::new();
        row.set_title(etiket);
        row.set_subtitle(deger);
        group.add(&row);
    }

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);
    vbox.append(&group);

    let btn_kapat = gtk4::Button::with_label("Kapat");
    btn_kapat.set_halign(gtk4::Align::End);
    btn_kapat.connect_clicked({
        let dialog = dialog.clone();
        move |_| { dialog.close(); }
    });
    vbox.append(&btn_kapat);

    dialog.set_child(Some(&vbox));
    dialog.present(Some(parent));
}

// ── Dışa Aktar ───────────────────────────────────────────────────────────────

/// Snapshot'ı .rxsnap dosyasına dışa aktar — dosya seçici açar.
pub fn show_export_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("RollbackX Snapshot (*.rxsnap)"));
    filter.add_pattern("*.rxsnap");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let file_dialog = gtk4::FileDialog::builder()
        .title("Snapshot'ı Dışa Aktar")
        .initial_name(format!("rollbackx_snapshot_{id}.rxsnap"))
        .filters(&filters)
        .build();

    let parent_c      = parent.clone();
    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));

    file_dialog.save(Some(&parent), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let path_str = path.to_string_lossy().to_string();

        let args = vec![
            "arsiv".to_string(), "disa-aktar".to_string(),
            id.to_string(),
            "--cikti".to_string(), path_str.clone(),
        ];
        let taken = on_done_cell.borrow_mut().take();
        run_with_progress(
            parent_c.clone(),
            "Snapshot dışa aktarılıyor...",
            args,
            move || { if let Some(f) = &taken { f(); } },
            move |result, win| match result {
                Ok(_) => show_toast(&format!("Snapshot #{id} dışa aktarıldı → {path_str}")),
                Err(e) => show_result_dialog(win, "Hata", &format!("Dışa aktarma başarısız:\n{e}"), true),
            },
        );
    });
}

// ── İçe Aktar ────────────────────────────────────────────────────────────────

/// .rxsnap dosyasını sisteme içe aktar — dosya seçici açar.
pub fn show_import_dialog(parent: gtk4::Window, on_done: impl Fn() + 'static) {
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("RollbackX Snapshot (*.rxsnap)"));
    filter.add_pattern("*.rxsnap");
    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let file_dialog = gtk4::FileDialog::builder()
        .title("Snapshot İçe Aktar (.rxsnap)")
        .filters(&filters)
        .build();

    let parent_c      = parent.clone();
    let on_done_cell: OnDoneCell =
        Rc::new(RefCell::new(Some(Box::new(on_done))));

    file_dialog.open(Some(&parent), gio::Cancellable::NONE, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let path_str = path.to_string_lossy().to_string();

        let args = vec![
            "arsiv".to_string(), "ice-aktar".to_string(),
            path_str,
        ];
        let taken = on_done_cell.borrow_mut().take();
        run_with_progress_bar(
            parent_c.clone(),
            "Snapshot içe aktarılıyor...",
            args,
            move || { if let Some(f) = &taken { f(); } },
            |result, win| match result {
                Ok(_) => show_toast("Snapshot başarıyla içe aktarıldı ve listeye eklendi."),
                Err(e) => show_result_dialog(win, "Hata", &format!("İçe aktarma başarısız:\n{e}"), true),
            },
        );
    });
}

// ── Ayarlar (APT Hook) ────────────────────────────────────────────────────

const APT_HOOK_PATH: &str = "/etc/apt/apt.conf.d/80rollbackx";

/// Ayarlar dialog'u — şu an sadece APT hook toggle'ı içerir.
pub fn show_settings_dialog(parent: gtk4::Window) {
    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Ayarlar");
    dialog.set_content_width(420);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    // ── APT Hook grubu ──
    let group = libadwaita::PreferencesGroup::new();
    group.set_title("APT Entegrasyonu");
    group.set_description(Some(
        "Etkinleştirilirse her 'apt install / upgrade' öncesinde otomatik snapshot alınır.",
    ));

    let hook_row = libadwaita::SwitchRow::new();
    hook_row.set_title("APT Hook");
    hook_row.set_subtitle("Paket kurulumundan önce otomatik snapshot");

    // Mevcut durumu oku
    let aktif = std::path::Path::new(APT_HOOK_PATH).exists();
    hook_row.set_active(aktif);

    group.add(&hook_row);
    vbox.append(&group);

    // Durum etiketi
    let durum_label = gtk4::Label::new(None);
    durum_label.add_css_class("dim-label");
    durum_label.add_css_class("caption");
    durum_label.set_halign(gtk4::Align::Start);
    durum_label.set_text(if aktif {
        "APT hook etkin — /etc/apt/apt.conf.d/80rollbackx"
    } else {
        "APT hook devre dışı"
    });
    vbox.append(&durum_label);

    // Kapat butonu
    let btn_kapat = gtk4::Button::with_label("Kapat");
    btn_kapat.set_halign(gtk4::Align::End);
    btn_kapat.connect_clicked({
        let dialog = dialog.clone();
        move |_| { dialog.close(); }
    });
    vbox.append(&btn_kapat);

    dialog.set_child(Some(&vbox));

    // Toggle değişince pkexec ile aç/kapat
    hook_row.connect_active_notify({
        let parent      = parent.clone();
        let durum_label = durum_label.clone();
        move |row| {
            let yeni_durum = row.is_active();
            let args = if yeni_durum {
                vec!["ayarlar".to_string(), "apt-hook".to_string(), "ac".to_string()]
            } else {
                vec!["ayarlar".to_string(), "apt-hook".to_string(), "kapat".to_string()]
            };

            let mesaj = if yeni_durum {
                "APT hook etkinleştiriliyor..."
            } else {
                "APT hook devre dışı bırakılıyor..."
            };

            let durum_label_c = durum_label.clone();
            let row_c         = row.clone();

            run_with_progress(
                parent.clone(),
                mesaj,
                args,
                || {},
                move |result, win| match result {
                    Ok(_) => {
                        let aktif = std::path::Path::new(APT_HOOK_PATH).exists();
                        durum_label_c.set_text(if aktif {
                            "APT hook etkin — /etc/apt/apt.conf.d/80rollbackx"
                        } else {
                            "APT hook devre dışı"
                        });
                        // Gerçek durumu yansıt (işlem başarısız olmuş olabilir)
                        row_c.set_active(aktif);
                        let _ = win; // kullanılmıyor ama tip eşlemesi için
                    }
                    Err(e) => {
                        // Hata olursa toggle'ı geri al
                        row_c.set_active(!yeni_durum);
                        show_result_dialog(win, "Hata", &format!("APT hook değiştirilemedi:\n{e}"), true);
                    }
                },
            );
        }
    });

    dialog.present(Some(&parent));
}

// ── Zamanlayıcı ──────────────────────────────────────────────────────────────

/// Zamanlayıcı kurma/gösterme/kaldırma dialog'u.
/// `snapshots`: mevcut snapshot ID ve isimleri listesi.
pub fn show_schedule_dialog(
    parent:    gtk4::Window,
    snapshots: Vec<(u32, String)>,
    on_done:   impl Fn() + 'static,
) {
    // Rc ile wrap et — birden fazla closure arasında paylaşılabilir hale getir
    let on_done_shared: Rc<dyn Fn() + 'static> = Rc::new(on_done);

    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Zamanlanmış Geri Yükleme");
    dialog.set_content_width(480);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    // ── Mevcut zamanlayıcı bilgisi ──
    if let Ok(Some(mevcut)) = Schedule::yukle() {
        let banner = libadwaita::Banner::new(&format!(
            "Aktif zamanlayıcı: Snapshot #{} — {}",
            mevcut.snapshot_id,
            mevcut.acikla()
        ));
        banner.set_revealed(true);
        banner.set_button_label(Some("Kaldır"));
        let parent_c  = parent.clone();
        let dialog_c  = dialog.clone();
        let od        = Rc::clone(&on_done_shared);
        let od_cell: OnDoneRcCell =
            Rc::new(RefCell::new(Some(od)));
        banner.connect_button_clicked(move |_| {
            let args      = vec!["zamanlayici".to_string(), "kaldir".to_string()];
            let taken     = od_cell.borrow_mut().take();
            let dialog_cc = dialog_c.clone();
            run_with_progress(
                parent_c.clone(),
                "Zamanlayıcı kaldırılıyor...",
                args,
                move || { dialog_cc.close(); if let Some(f) = &taken { f(); } },
                |result, win| match result {
                    Ok(_)  => show_toast("Zamanlayıcı devre dışı bırakıldı."),
                    Err(e) => show_result_dialog(win, "Hata", &e, true),
                },
            );
        });
        vbox.append(&banner);
    }

    // ── Snapshot seçici ──
    let snap_group = libadwaita::PreferencesGroup::new();
    snap_group.set_title("Ayarlar");

    let snap_row = libadwaita::ComboRow::new();
    snap_row.set_title("Snapshot");
    let snap_model = gtk4::StringList::new(&[]);
    let mut snap_ids: Vec<u32> = Vec::new();
    for (id, name) in &snapshots {
        snap_model.append(&format!("#{id} — {name}"));
        snap_ids.push(*id);
    }
    snap_row.set_model(Some(&snap_model));

    // ── Sıklık ──
    let siklik_row = libadwaita::ComboRow::new();
    siklik_row.set_title("Sıklık");
    let siklik_model = gtk4::StringList::new(&["Günlük", "Haftalık", "Aylık"]);
    siklik_row.set_model(Some(&siklik_model));

    // ── Gün ──
    let gun_row = libadwaita::ComboRow::new();
    gun_row.set_title("Gün");
    let gun_model = gtk4::StringList::new(&[
        "Pazartesi", "Salı", "Çarşamba", "Perşembe", "Cuma", "Cumartesi", "Pazar",
    ]);
    let gun_values = ["pazartesi", "sali", "carsamba", "persembe", "cuma", "cumartesi", "pazar"];
    gun_row.set_model(Some(&gun_model));

    // ── Kaçıncı hafta ──
    let kacinci_row = libadwaita::ComboRow::new();
    kacinci_row.set_title("Kaçıncı hafta");
    let kacinci_model = gtk4::StringList::new(&["İlk", "İkinci", "Üçüncü", "Dördüncü"]);
    kacinci_row.set_model(Some(&kacinci_model));

    // ── Saat ──
    let saat_row = libadwaita::EntryRow::new();
    saat_row.set_title("Saat (HH:MM)");
    saat_row.set_text("03:00");

    snap_group.add(&snap_row);
    snap_group.add(&siklik_row);
    snap_group.add(&gun_row);
    snap_group.add(&kacinci_row);
    snap_group.add(&saat_row);
    vbox.append(&snap_group);

    // Sıklık değişince gün/kacinci satırlarını göster/gizle
    {
        let gun_row_c     = gun_row.clone();
        let kacinci_row_c = kacinci_row.clone();
        siklik_row.connect_selected_notify(move |row| {
            let idx = row.selected();
            gun_row_c.set_visible(idx == 1 || idx == 2);
            kacinci_row_c.set_visible(idx == 2);
        });
        gun_row.set_visible(false);
        kacinci_row.set_visible(false);
    }

    // ── Butonlar ──
    let btn_box   = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let btn_iptal = gtk4::Button::with_label("İptal");
    let btn_kur   = gtk4::Button::with_label("Zamanlayıcıyı Kur");
    btn_kur.add_css_class("suggested-action");
    btn_box.append(&btn_iptal);
    btn_box.append(&btn_kur);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));

    btn_iptal.connect_clicked({
        let dialog = dialog.clone();
        move |_| { dialog.close(); }
    });

    // on_done_shared → btn_kur için ayrı bir cell
    let od_kur       = Rc::clone(&on_done_shared);
    let on_done_cell: OnDoneRcCell =
        Rc::new(RefCell::new(Some(od_kur)));

    btn_kur.connect_clicked({
        let dialog       = dialog.clone();
        let snap_row     = snap_row.clone();
        let siklik_row   = siklik_row.clone();
        let gun_row      = gun_row.clone();
        let kacinci_row  = kacinci_row.clone();
        let saat_row     = saat_row.clone();
        let parent       = parent.clone();
        let on_done_cell = on_done_cell.clone();
        let snap_ids     = snap_ids.clone();

        move |_| {
            let idx = snap_row.selected() as usize;
            if snap_ids.is_empty() || idx >= snap_ids.len() {
                show_result_dialog(&parent, "Hata", "Lütfen bir snapshot seçin.", true);
                return;
            }
            let snap_id    = snap_ids[idx];
            let siklik_idx = siklik_row.selected();
            let siklik_str = match siklik_idx {
                0 => "gunluk",
                1 => "haftalik",
                _ => "aylik",
            };

            let mut args = vec![
                "zamanlayici".to_string(), "kur".to_string(),
                snap_id.to_string(),
                "--siklik".to_string(), siklik_str.to_string(),
                "--saat".to_string(), saat_row.text().to_string(),
            ];

            if siklik_idx >= 1 {
                let gi  = gun_row.selected() as usize;
                let gun = gun_values.get(gi).copied().unwrap_or("pazartesi");
                args.push("--gun".to_string());
                args.push(gun.to_string());
            }
            if siklik_idx == 2 {
                let n = kacinci_row.selected() + 1;
                args.push("--kacinci".to_string());
                args.push(n.to_string());
            }

            dialog.close();
            let taken = on_done_cell.borrow_mut().take();
            run_with_progress(
                parent.clone(),
                "Zamanlayıcı kuruluyor...",
                args,
                move || { if let Some(f) = &taken { f(); } },
                |result, win| match result {
                    Ok(_)  => show_toast("Zamanlanmış geri yükleme aktif."),
                    Err(e) => show_result_dialog(win, "Hata", &e, true),
                },
            );
        }
    });

    dialog.present(Some(&parent));
}

// ── Önyükleme Otomatik Geri Yükleme ──────────────────────────────────────────

/// Her açılışta otomatik geri yükleme kurma/kaldırma dialog'u.
/// Yalnızca Rsync snapshot'ları desteklenir.
pub fn show_auto_restore_dialog(
    parent:  gtk4::Window,
    on_done: impl Fn() + 'static,
) {
    let on_done_shared: Rc<dyn Fn() + 'static> = Rc::new(on_done);

    let dialog = libadwaita::Dialog::new();
    dialog.set_title("Önyükleme Otomatik Geri Yükleme");
    dialog.set_content_width(480);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    // ── Mevcut durum banner'ı ──
    const AUTO_CONF: &str = "/var/lib/rollbackx/auto-restore.conf";
    if std::path::Path::new(AUTO_CONF).exists() {
        let conf_content = std::fs::read_to_string(AUTO_CONF)
            .unwrap_or_default()
            .trim()
            .to_string();
        let isim = load_snapshots()
            .ok()
            .unwrap_or_default()
            .iter()
            .find(|s| conf_content.contains(&s.backend_ref))
            .map(|s| format!("#{} \"{}\"", s.id, s.name))
            .unwrap_or_else(|| conf_content.clone());
        let banner = libadwaita::Banner::new(&format!(
            "Aktif: Her açılışta {} geri yükleniyor", isim
        ));
        banner.set_revealed(true);
        banner.set_button_label(Some("Kaldır"));
        let parent_c = parent.clone();
        let dialog_c = dialog.clone();
        let od = Rc::clone(&on_done_shared);
        let od_cell: OnDoneRcCell =
            Rc::new(RefCell::new(Some(od)));
        banner.connect_button_clicked(move |_| {
            let args      = vec!["oto".to_string(), "kaldir".to_string()];
            let taken     = od_cell.borrow_mut().take();
            let dialog_cc = dialog_c.clone();
            run_with_progress(
                parent_c.clone(),
                "Otomatik geri yükleme kaldırılıyor...",
                args,
                move || { dialog_cc.close(); if let Some(f) = &taken { f(); } },
                |result, win| match result {
                    Ok(_)  => show_toast("Önyükleme otomatik geri yükleme devre dışı."),
                    Err(e) => show_result_dialog(win, "Hata", &e, true),
                },
            );
        });
        vbox.append(&banner);
    }

    // ── Açıklama ──
    let aciklama = gtk4::Label::new(Some(
        "Seçilen snapshot, bilgisayar her açıldığında otomatik olarak geri yüklenir.\n\
         Yalnızca Rsync snapshot'ları desteklenir.",
    ));
    aciklama.set_wrap(true);
    aciklama.set_xalign(0.0);
    vbox.append(&aciklama);

    // ── Snapshot seçici (sadece Rsync) ──
    let snap_group = libadwaita::PreferencesGroup::new();
    snap_group.set_title("Snapshot Seç");

    let snap_row = libadwaita::ComboRow::new();
    snap_row.set_title("Snapshot");
    let snap_model = gtk4::StringList::new(&[]);
    let mut snap_ids: Vec<u32> = Vec::new();
    if let Ok(snaps) = load_snapshots() {
        for s in &snaps {
            if s.backend == BackendKind::Rsync {
                snap_model.append(&format!("#{} — {}", s.id, s.name));
                snap_ids.push(s.id);
            }
        }
    }
    snap_row.set_model(Some(&snap_model));
    snap_group.add(&snap_row);
    vbox.append(&snap_group);

    // ── Butonlar ──
    let btn_box   = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);
    let btn_iptal = gtk4::Button::with_label("İptal");
    let btn_kur   = gtk4::Button::with_label("Otomatik Geri Yüklemeyi Kur");
    btn_kur.add_css_class("suggested-action");
    btn_box.append(&btn_iptal);
    btn_box.append(&btn_kur);
    vbox.append(&btn_box);

    dialog.set_child(Some(&vbox));

    btn_iptal.connect_clicked({
        let dialog = dialog.clone();
        move |_| { dialog.close(); }
    });

    let od_kur: OnDoneRcCell =
        Rc::new(RefCell::new(Some(Rc::clone(&on_done_shared))));

    btn_kur.connect_clicked({
        let dialog       = dialog.clone();
        let snap_row     = snap_row.clone();
        let parent       = parent.clone();
        let on_done_cell = od_kur.clone();
        let snap_ids     = snap_ids.clone();
        move |_| {
            let idx = snap_row.selected() as usize;
            if snap_ids.is_empty() || idx >= snap_ids.len() {
                show_result_dialog(&parent, "Hata", "Lütfen bir Rsync snapshot seçin.", true);
                return;
            }
            let snap_id = snap_ids[idx];
            let args = vec!["oto".to_string(), "kur".to_string(), snap_id.to_string()];
            let taken = on_done_cell.borrow_mut().take();
            dialog.close();
            run_with_progress(
                parent.clone(),
                "Otomatik geri yükleme kuruluyor...",
                args,
                move || { if let Some(f) = &taken { f(); } },
                |result, win| match result {
                    Ok(_)  => show_toast("Her açılışta bu snapshot geri yüklenecek."),
                    Err(e) => show_result_dialog(win, "Hata", &e, true),
                },
            );
        }
    });

    dialog.present(Some(&parent));
}

// ── Snapshot Yeniden Adlandırma ───────────────────────────────────────────────

pub fn show_rename_dialog(
    parent:  gtk4::Window,
    snap:    &Snapshot,
    on_done: impl Fn() + 'static,
) {
    let snap_id    = snap.id;
    let mevcut_ad  = snap.name.clone();
    let mevcut_ac  = snap.description.clone().unwrap_or_default();
    let on_done_rc: Rc<dyn Fn()> = Rc::new(on_done);

    let dialog = libadwaita::Dialog::new();
    dialog.set_title(&format!("Snapshot #{snap_id} Düzenle"));
    dialog.set_content_width(420);

    let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(24);
    vbox.set_margin_end(24);

    let group = libadwaita::PreferencesGroup::new();
    group.set_title("Snapshot Bilgisi");

    let isim_row = libadwaita::EntryRow::new();
    isim_row.set_title("İsim");
    isim_row.set_text(&mevcut_ad);

    let aciklama_row = libadwaita::EntryRow::new();
    aciklama_row.set_title("Açıklama (opsiyonel)");
    aciklama_row.set_text(&mevcut_ac);

    group.add(&isim_row);
    group.add(&aciklama_row);

    let btn_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    btn_box.set_halign(gtk4::Align::End);

    let btn_iptal  = gtk4::Button::builder().label("İptal").build();
    let btn_kaydet = gtk4::Button::builder()
        .label("Kaydet")
        .css_classes(["suggested-action"])
        .build();

    btn_box.append(&btn_iptal);
    btn_box.append(&btn_kaydet);

    vbox.append(&group);
    vbox.append(&btn_box);
    dialog.set_child(Some(&vbox));

    let dialog_c = dialog.clone();
    btn_iptal.connect_clicked(move |_| { dialog_c.close(); });

    let dialog_c       = dialog.clone();
    let isim_row_c     = isim_row.clone();
    let aciklama_row_c = aciklama_row.clone();
    let parent_c       = parent.clone();

    btn_kaydet.connect_clicked(move |_| {
        let yeni_ad = isim_row_c.text().trim().to_string();
        if yeni_ad.is_empty() {
            return;
        }
        let yeni_ac = aciklama_row_c.text().trim().to_string();

        let mut args = vec![
            "snapshot".to_string(),
            "yeniden-adlandir".to_string(),
            snap_id.to_string(),
            yeni_ad,
        ];
        if !yeni_ac.is_empty() {
            args.push("--aciklama".to_string());
            args.push(yeni_ac);
        }

        dialog_c.close();
        let od = Rc::clone(&on_done_rc);
        run_with_progress(
            parent_c.clone(),
            "Snapshot güncelleniyor...",
            args,
            move || od(),
            |result, win| match result {
                Ok(_)  => show_toast("Snapshot güncellendi."),
                Err(e) => show_result_dialog(win, "Hata", &e, true),
            },
        );
    });

    dialog.present(Some(&parent));
}

// ── Snapshot Doğrulama ────────────────────────────────────────────────────────

/// Snapshot sistem dosyalarını doğrular, sonuçları bir dialog'da gösterir.
pub fn show_verify_dialog(parent: gtk4::Window, id: u32) {
    let args = vec![
        "snapshot".to_string(),
        "dogrula".to_string(),
        id.to_string(),
    ];

    run_with_progress(
        parent.clone(),
        "Snapshot doğrulanıyor...",
        args,
        || {},
        move |result, win| match result {
            Ok(cikti) => {
                let dialog = libadwaita::Dialog::new();
                dialog.set_title(&format!("Snapshot #{id} — Doğrulama Sonucu"));
                dialog.set_content_width(460);

                let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
                vbox.set_margin_top(24);
                vbox.set_margin_bottom(16);
                vbox.set_margin_start(24);
                vbox.set_margin_end(24);

                // Başarı/başarısız banner
                let tumu_basarili = !cikti.contains('✗');
                let banner_text = if tumu_basarili {
                    "Doğrulama başarılı — tüm kontroller geçti"
                } else {
                    "Bazı kontroller başarısız — snapshot bozuk olabilir"
                };
                let banner = libadwaita::Banner::new(banner_text);
                banner.set_revealed(true);

                // Sonuç metni
                let text_view = gtk4::TextView::new();
                text_view.set_editable(false);
                text_view.set_cursor_visible(false);
                text_view.set_monospace(true);
                text_view.set_wrap_mode(gtk4::WrapMode::Word);
                text_view.buffer().set_text(cikti.trim());

                let scrolled = gtk4::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk4::PolicyType::Never)
                    .vscrollbar_policy(gtk4::PolicyType::Automatic)
                    .min_content_height(160)
                    .child(&text_view)
                    .build();

                let btn_kapat = gtk4::Button::builder()
                    .label("Kapat")
                    .halign(gtk4::Align::End)
                    .build();
                let dialog_c = dialog.clone();
                btn_kapat.connect_clicked(move |_| { dialog_c.close(); });

                vbox.append(&banner);
                vbox.append(&scrolled);
                vbox.append(&btn_kapat);
                dialog.set_child(Some(&vbox));
                dialog.present(Some(win));
            }
            Err(e) => {
                show_result_dialog(win, "Doğrulama Hatası", &e, true);
            }
        },
    );
}

// ── Dosya Geri Yükle ────────────────────────────────────────────────────────

/// Snapshot'ı mount edip dosya seçtirip geri yükleyen dialog.
pub fn show_file_restore_dialog(parent: gtk4::Window, id: u32, on_done: impl Fn() + 'static) {
    let on_done = Rc::new(on_done);

    // Önce snapshot'ı mount et
    let args = vec!["snapshot".to_string(), "bagla".to_string(), id.to_string()];
    let on_done_c = on_done.clone();
    run_with_progress(
        parent.clone(),
        "Snapshot bağlanıyor...",
        args,
        move || {},
        move |result, win| {
            match result {
                Err(e) => {
                    // "zaten bağlı" hatasını yut
                    if !e.contains("zaten") {
                        show_result_dialog(win, "Hata", &format!("Snapshot bağlanamadı:\n{e}"), true);
                        return;
                    }
                }
                Ok(_) => {}
            }

            // Mount başarılı — dosya seçici aç
            let mount_path = format!("/mnt/rollbackx/{id}");
            let file_dialog = gtk4::FileDialog::new();
            file_dialog.set_title("Geri yüklenecek dosya/klasör seçin");
            file_dialog.set_modal(true);
            let initial = gtk4::gio::File::for_path(&mount_path);
            file_dialog.set_initial_folder(Some(&initial));

            let win_c = win.clone();
            let on_done_inner = on_done_c.clone();
            file_dialog.select_multiple_folders(Some(win), gtk4::gio::Cancellable::NONE, move |result| {
                match result {
                    Err(_) => {
                        // Kullanıcı iptal etti — dosya seçmeyi de dene
                        let file_dialog2 = gtk4::FileDialog::new();
                        file_dialog2.set_title("Geri yüklenecek dosyaları seçin");
                        file_dialog2.set_modal(true);
                        let initial2 = gtk4::gio::File::for_path(&format!("/mnt/rollbackx/{id}"));
                        file_dialog2.set_initial_folder(Some(&initial2));
                        let win_c2 = win_c.clone();
                        let mount_path2 = format!("/mnt/rollbackx/{id}");
                        let on_done_2 = on_done_inner.clone();
                        file_dialog2.open_multiple(Some(&win_c), gtk4::gio::Cancellable::NONE, move |result| {
                            match result {
                                Err(_) => {} // Kullanıcı iptal etti
                                Ok(files) => {
                                    let yollar = files_to_restore_paths(&files, &mount_path2);
                                    if !yollar.is_empty() {
                                        do_file_restore(win_c2, id, yollar, on_done_2);
                                    }
                                }
                            }
                        });
                    }
                    Ok(folders) => {
                        let yollar = files_to_restore_paths(&folders, &mount_path);
                        if !yollar.is_empty() {
                            do_file_restore(win_c, id, yollar, on_done_inner);
                        }
                    }
                }
            });
        },
    );
}

fn files_to_restore_paths(files: &gtk4::gio::ListModel, mount_prefix: &str) -> Vec<String> {
    let mut yollar = Vec::new();
    for i in 0..files.n_items() {
        if let Some(obj) = files.item(i) {
            if let Some(file) = obj.downcast_ref::<gtk4::gio::File>() {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    // /mnt/rollbackx/{id}/etc/fstab → /etc/fstab
                    if let Some(rest) = path_str.strip_prefix(mount_prefix) {
                        if !rest.is_empty() {
                            yollar.push(rest.to_string());
                        }
                    }
                }
            }
        }
    }
    yollar
}

fn do_file_restore(parent: gtk4::Window, id: u32, yollar: Vec<String>, on_done: Rc<dyn Fn()>) {
    let mut args = vec![
        "snapshot".to_string(),
        "dosya-yukle".to_string(),
        id.to_string(),
    ];
    args.extend(yollar.iter().cloned());

    let yol_listesi = yollar.join("\n• ");

    // Onay dialogu
    let dialog = libadwaita::AlertDialog::new(
        Some("Dosya Geri Yükle"),
        Some(&format!(
            "Seçilen {} dosya/klasör snapshot #{id}'den geri yüklenecek:\n\n• {}\n\nMevcut dosyalar üzerine yazılacak. Devam edilsin mi?",
            yollar.len(),
            yol_listesi
        )),
    );
    dialog.add_responses(&[("iptal", "İptal"), ("yukle", "Geri Yükle")]);
    dialog.set_response_appearance("yukle", libadwaita::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("iptal"));

    let parent_c = parent.clone();
    dialog.connect_response(None, move |dlg, response| {
        dlg.close();
        if response != "yukle" {
            return;
        }
        let on_done_c = on_done.clone();
        run_with_progress(
            parent_c.clone(),
            "Dosyalar geri yükleniyor...",
            args.clone(),
            move || { on_done_c(); },
            |result, win| match result {
                Ok(msg) => {
                    show_toast("Dosyalar geri yüklendi.");
                    if !msg.trim().is_empty() {
                        show_result_dialog(win, "Sonuç", &msg, false);
                    }
                }
                Err(e) => show_result_dialog(win, "Hata", &e, true),
            },
        );
    });
    dialog.present(Some(&parent));
}
