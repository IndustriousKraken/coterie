//! In-memory fakes for `SystemCommand` and `FileSystem` used by
//! integration tests (and the `--dry-run` plumbing).

use crate::fs_ops::FileSystem;
use crate::system::{CommandOutput, SystemCommand};
use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCall {
    pub cmd: String,
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
    pub interactive: bool,
}

pub struct FakeSystem {
    pub responses: RefCell<HashMap<String, CommandOutput>>,
    pub calls: RefCell<Vec<RecordedCall>>,
}

impl FakeSystem {
    pub fn new() -> Self {
        Self {
            responses: RefCell::new(HashMap::new()),
            calls: RefCell::new(Vec::new()),
        }
    }

    fn key(cmd: &str, args: &[&str]) -> String {
        let mut k = String::from(cmd);
        for a in args {
            k.push(' ');
            k.push_str(a);
        }
        k
    }

    pub fn respond(&self, cmd: &str, args: &[&str], output: CommandOutput) {
        self.responses
            .borrow_mut()
            .insert(Self::key(cmd, args), output);
    }

    pub fn calls_for(&self, cmd: &str) -> Vec<RecordedCall> {
        self.calls
            .borrow()
            .iter()
            .filter(|c| c.cmd == cmd)
            .cloned()
            .collect()
    }

    fn record(&self, cmd: &str, args: &[&str], stdin: Option<&[u8]>, interactive: bool) {
        self.calls.borrow_mut().push(RecordedCall {
            cmd: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            stdin: stdin.map(|b| b.to_vec()),
            interactive,
        });
    }

    fn lookup(&self, cmd: &str, args: &[&str]) -> CommandOutput {
        let key = Self::key(cmd, args);
        if let Some(o) = self.responses.borrow().get(&key) {
            return o.clone();
        }
        // Default: empty success.
        CommandOutput::ok("")
    }
}

impl Default for FakeSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemCommand for FakeSystem {
    fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        self.record(cmd, args, None, false);
        Ok(self.lookup(cmd, args))
    }

    fn run_with_stdin(&self, cmd: &str, args: &[&str], stdin: &[u8]) -> Result<CommandOutput> {
        self.record(cmd, args, Some(stdin), false);
        Ok(self.lookup(cmd, args))
    }

    fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
        self.record(cmd, args, None, true);
        Ok(self.lookup(cmd, args))
    }
}

// ---------------------------------------------------------------------
// FakeFs
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsOp {
    Read(PathBuf),
    Write(PathBuf, Vec<u8>),
    Append(PathBuf, Vec<u8>),
    Mkdirp(PathBuf),
    Chmod(PathBuf, u32),
    Chown(PathBuf, String, String),
    Rename(PathBuf, PathBuf),
    Remove(PathBuf),
    RemoveDir(PathBuf),
}

pub struct FakeFs {
    pub files: RefCell<HashMap<PathBuf, Vec<u8>>>,
    pub dirs: RefCell<std::collections::HashSet<PathBuf>>,
    pub ops: RefCell<Vec<FsOp>>,
}

impl FakeFs {
    pub fn new() -> Self {
        Self {
            files: RefCell::new(HashMap::new()),
            dirs: RefCell::new(std::collections::HashSet::new()),
            ops: RefCell::new(Vec::new()),
        }
    }

    pub fn seed_file(&self, path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) {
        self.files.borrow_mut().insert(path.into(), contents.into());
    }

    pub fn seed_dir(&self, path: impl Into<PathBuf>) {
        self.dirs.borrow_mut().insert(path.into());
    }

    pub fn snapshot(&self, path: &Path) -> Option<Vec<u8>> {
        self.files.borrow().get(path).cloned()
    }

    pub fn snapshot_string(&self, path: &Path) -> Option<String> {
        self.snapshot(path).and_then(|b| String::from_utf8(b).ok())
    }
}

impl Default for FakeFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for FakeFs {
    fn read_to_string(&self, path: &Path) -> Result<String> {
        self.ops.borrow_mut().push(FsOp::Read(path.to_path_buf()));
        match self.files.borrow().get(path) {
            Some(b) => Ok(String::from_utf8_lossy(b).into_owned()),
            None => Err(anyhow!("FakeFs: no such file {}", path.display())),
        }
    }

    fn write(&self, path: &Path, contents: &[u8]) -> Result<()> {
        self.ops
            .borrow_mut()
            .push(FsOp::Write(path.to_path_buf(), contents.to_vec()));
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), contents.to_vec());
        Ok(())
    }

    fn append(&self, path: &Path, contents: &[u8]) -> Result<()> {
        self.ops
            .borrow_mut()
            .push(FsOp::Append(path.to_path_buf(), contents.to_vec()));
        let mut files = self.files.borrow_mut();
        let entry = files.entry(path.to_path_buf()).or_default();
        entry.extend_from_slice(contents);
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> Result<()> {
        self.ops.borrow_mut().push(FsOp::Mkdirp(path.to_path_buf()));
        self.dirs.borrow_mut().insert(path.to_path_buf());
        Ok(())
    }

    fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        self.ops
            .borrow_mut()
            .push(FsOp::Chmod(path.to_path_buf(), mode));
        Ok(())
    }

    fn chown(&self, path: &Path, user: &str, group: &str) -> Result<()> {
        self.ops.borrow_mut().push(FsOp::Chown(
            path.to_path_buf(),
            user.to_string(),
            group.to_string(),
        ));
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.borrow().contains(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.ops
            .borrow_mut()
            .push(FsOp::Rename(from.to_path_buf(), to.to_path_buf()));
        let mut files = self.files.borrow_mut();
        if let Some(data) = files.remove(from) {
            files.insert(to.to_path_buf(), data);
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<()> {
        self.ops.borrow_mut().push(FsOp::Remove(path.to_path_buf()));
        self.files.borrow_mut().remove(path);
        Ok(())
    }

    fn remove_dir_all(&self, path: &Path) -> Result<()> {
        self.ops
            .borrow_mut()
            .push(FsOp::RemoveDir(path.to_path_buf()));
        self.dirs.borrow_mut().remove(path);
        Ok(())
    }
}
