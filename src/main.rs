use disk_map::DiskMapApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DiskMap")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "DiskMap",
        options,
        Box::new(|_cc| Ok(Box::<DiskMapApp>::default())),
    )
}
