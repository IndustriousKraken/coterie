use anyhow::{Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub trait FileSystem {
    fn read_to_string(&self, path: &Path) -> Result<String>;
    fn write(&self, path: &Path, contents: &[u8]) -> Result<()>;
    fn append(&self, path: &Path, contents: &[u8]) -> Result<()>;
    fn create_dir_all(&self, path: &Path) -> Result<()>;
    fn chmod(&self, path: &Path, mode: u32) -> Result<()>;
    fn chown(&self, path: &Path, user: &str, group: &str) -> Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove_file(&self, path: &Path) -> Result<()>;
    fn remove_dir_all(&self, path: &Path) -> Result<()>;
}

pub struct RealFs;

impl FileSystem for RealFs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
    }

    fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create dir {}", parent.display()))?;
            }
        }
        fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
    }

    fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("failed to open {} for append", path.display()))?;
        f.write_all(contents)
            .with_context(|| format!("failed to append to {}", path.display()))?;
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path).with_context(|| format!("failed to create dir {}", path.display()))
    }

    fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .with_context(|| format!("failed to chmod {} to {:o}", path.display(), mode))
    }

    fn chown(&self, path: &Path, user: &str, group: &str) -> Result<()> {
        let target = format!("{user}:{group}");
        let status = std::process::Command::new("chown")
            .arg("-R")
            .arg(&target)
            .arg(path)
            .status()
            .with_context(|| format!("failed to spawn chown for {}", path.display()))?;
        if !status.success() {
            anyhow::bail!(
                "chown {} {} returned {}",
                target,
                path.display(),
                status.code().unwrap_or(-1)
            );
        }
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        fs::rename(from, to)
            .with_context(|| format!("failed to rename {} → {}", from.display(), to.display()))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove dir {}", path.display()))
    }
}
