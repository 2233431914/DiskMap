use std::path::Path;

pub fn reveal_in_finder(path: &Path) {
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
}

pub fn open_path(path: &Path) {
    let _ = open::that(path);
}

pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    let url = format!("trash://{}", path.display());
    open::that(&url)?;
    Ok(())
}
