use disk_lens::DiskLensApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DiskLens")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "DiskLens",
        options,
        Box::new(|_cc| Ok(Box::<DiskLensApp>::default())),
    )
}
