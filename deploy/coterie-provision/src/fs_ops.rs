use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

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

impl RealFs {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for RealFs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        std::fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
    }

    fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .with_context(|| format!("failed to open {} for append", path.display()))?;
        f.write_all(contents)
            .with_context(|| format!("failed to append to {}", path.display()))?;
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)
            .with_context(|| format!("failed to create directory {}", path.display()))
    }

    fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        let perms = std::fs::Permissions::from_mode(mode);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("failed to chmod {}", path.display()))
    }

    fn chown(&self, path: &Path, user: &str, group: &str) -> Result<()> {
        // Shell out to chown to avoid a nix dependency.
        let spec = format!("{user}:{group}");
        let status = Command::new("chown")
            .arg("-R")
            .arg(&spec)
            .arg(path)
            .status()
            .with_context(|| format!("failed to spawn chown for {}", path.display()))?;
        if !status.success() {
            anyhow::bail!("chown {spec} {} failed", path.display());
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
        std::fs::rename(from, to)
            .with_context(|| format!("failed to rename {} -> {}", from.display(), to.display()))
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        std::fs::remove_file(path).with_context(|| format!("failed to remove {}", path.display()))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove dir {}", path.display()))
    }
}
