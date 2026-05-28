use disk_map::{configure_theme, DiskMapApp};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("disk-map")
            .with_title("DiskMap")
            .with_inner_size([1280.0, 800.0]),
        persist_window: true,
        ..Default::default()
    };

    eframe::run_native(
        "DiskMap",
        options,
        Box::new(|cc| {
            configure_theme(&cc.egui_ctx);
            Ok(Box::new(DiskMapApp::new(cc)))
        }),
    )
}
