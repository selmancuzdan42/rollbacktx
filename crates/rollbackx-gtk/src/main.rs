mod app;
mod dialogs;
mod snapshot_row;
mod tray;

use gtk4::prelude::*;
use std::sync::{Arc, Mutex};

fn main() {
    let arka_plan = std::env::args().any(|a| a == "--arka-plan" || a == "--background");

    let (tx, rx) = std::sync::mpsc::channel::<tray::TrayCmd>();
    let rx = Arc::new(Mutex::new(rx));

    tray::spawn_tray(tx);

    let app = libadwaita::Application::builder()
        .application_id("org.rollbackx.RollbackX")
        .flags(gio::ApplicationFlags::FLAGS_NONE)
        .build();

    app.connect_activate(move |app| {
        if let Some(win) = app.windows().into_iter().next() {
            win.present();
            return;
        }
        app::build_ui(app, arka_plan, Arc::clone(&rx));
    });

    let status = app.run();
    std::process::exit(status.into());
}
