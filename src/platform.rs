use std::path::Path;

pub fn reveal_in_finder(path: &Path) -> anyhow::Result<()> {
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    Ok(())
}

pub fn open_path(path: &Path) -> anyhow::Result<()> {
    open::that(path)?;
    Ok(())
}

pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    trash::delete(path)?;
    Ok(())
}
