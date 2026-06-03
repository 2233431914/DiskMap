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

#[cfg(test)]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(all(not(test), target_os = "macos"))]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};

    let mut context = trash::TrashContext::new();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context.delete(path)?;
    Ok(())
}

#[cfg(all(not(test), not(target_os = "macos")))]
pub fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    trash::delete(path)?;
    Ok(())
}
