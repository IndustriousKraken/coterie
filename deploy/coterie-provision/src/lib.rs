pub mod caddyfile;
pub mod checklist;
pub mod env_template;
pub mod fs_ops;
pub mod github_api;
pub mod install;
pub mod output;
pub mod prompts;
pub mod stripe_api;
pub mod stripe_check;
pub mod switch_to_live;
pub mod system;
pub mod version_selector;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use crate::fs_ops::FileSystem;
    use crate::prompts::Prompter;
    use crate::system::{CommandOutput, SystemCommand};
    use anyhow::{anyhow, Result};
    use secrecy::SecretString;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub struct CommandKey {
        pub cmd: String,
        pub args: Vec<String>,
    }

    impl CommandKey {
        pub fn new(cmd: &str, args: &[&str]) -> Self {
            Self {
                cmd: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct RecordedCall {
        pub cmd: String,
        pub args: Vec<String>,
        pub stdin: Option<Vec<u8>>,
        pub interactive: bool,
    }

    /// In-memory `SystemCommand` that records every invocation and
    /// looks up scripted responses. Unmatched commands default to
    /// `exit 0` with empty stdout/stderr.
    pub struct FakeSystem {
        pub calls: RefCell<Vec<RecordedCall>>,
        pub responses: RefCell<HashMap<CommandKey, CommandOutput>>,
        pub default_response: CommandOutput,
    }

    impl FakeSystem {
        pub fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(HashMap::new()),
                default_response: CommandOutput {
                    status: 0,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            }
        }

        pub fn respond_to(&self, cmd: &str, args: &[&str], out: CommandOutput) {
            self.responses
                .borrow_mut()
                .insert(CommandKey::new(cmd, args), out);
        }

        pub fn called_with(&self, cmd: &str, args: &[&str]) -> bool {
            self.calls.borrow().iter().any(|c| {
                c.cmd == cmd && c.args.iter().map(|s| s.as_str()).collect::<Vec<_>>() == args
            })
        }

        pub fn call_count(&self, cmd: &str) -> usize {
            self.calls.borrow().iter().filter(|c| c.cmd == cmd).count()
        }
    }

    impl Default for FakeSystem {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SystemCommand for FakeSystem {
        fn run(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(RecordedCall {
                cmd: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                stdin: None,
                interactive: false,
            });
            let key = CommandKey::new(cmd, args);
            let resp = self
                .responses
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| self.default_response.clone());
            Ok(resp)
        }

        fn run_with_stdin(
            &self,
            cmd: &str,
            args: &[&str],
            stdin_bytes: &[u8],
        ) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(RecordedCall {
                cmd: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                stdin: Some(stdin_bytes.to_vec()),
                interactive: false,
            });
            let key = CommandKey::new(cmd, args);
            let resp = self
                .responses
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| self.default_response.clone());
            Ok(resp)
        }

        fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(RecordedCall {
                cmd: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                stdin: None,
                interactive: true,
            });
            let key = CommandKey::new(cmd, args);
            let resp = self
                .responses
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| self.default_response.clone());
            Ok(resp)
        }
    }

    #[derive(Debug, Clone)]
    pub enum FsOp {
        Read(PathBuf),
        Write(PathBuf, Vec<u8>),
        Append(PathBuf, Vec<u8>),
        CreateDirAll(PathBuf),
        Chmod(PathBuf, u32),
        Chown(PathBuf, String, String),
        Rename(PathBuf, PathBuf),
        RemoveFile(PathBuf),
        RemoveDirAll(PathBuf),
    }

    /// In-memory filesystem backed by a `HashMap<PathBuf, Vec<u8>>`.
    pub struct FakeFs {
        pub files: RefCell<HashMap<PathBuf, Vec<u8>>>,
        pub dirs: RefCell<Vec<PathBuf>>,
        pub ops: RefCell<Vec<FsOp>>,
    }

    impl FakeFs {
        pub fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                dirs: RefCell::new(Vec::new()),
                ops: RefCell::new(Vec::new()),
            }
        }

        pub fn put(&self, path: &Path, contents: &[u8]) {
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), contents.to_vec());
        }

        pub fn get(&self, path: &Path) -> Option<Vec<u8>> {
            self.files.borrow().get(path).cloned()
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
            let files = self.files.borrow();
            let bytes = files
                .get(path)
                .ok_or_else(|| anyhow!("FakeFs: no file at {}", path.display()))?;
            Ok(String::from_utf8_lossy(bytes).into_owned())
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
            self.ops
                .borrow_mut()
                .push(FsOp::CreateDirAll(path.to_path_buf()));
            self.dirs.borrow_mut().push(path.to_path_buf());
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
            self.files.borrow().contains_key(path) || self.dirs.borrow().iter().any(|d| d == path)
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.borrow().iter().any(|d| d == path)
        }

        fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.ops
                .borrow_mut()
                .push(FsOp::Rename(from.to_path_buf(), to.to_path_buf()));
            let mut files = self.files.borrow_mut();
            if let Some(bytes) = files.remove(from) {
                files.insert(to.to_path_buf(), bytes);
                Ok(())
            } else {
                Err(anyhow!("FakeFs: no file at {}", from.display()))
            }
        }

        fn remove_file(&self, path: &Path) -> Result<()> {
            self.ops
                .borrow_mut()
                .push(FsOp::RemoveFile(path.to_path_buf()));
            self.files.borrow_mut().remove(path);
            Ok(())
        }

        fn remove_dir_all(&self, path: &Path) -> Result<()> {
            self.ops
                .borrow_mut()
                .push(FsOp::RemoveDirAll(path.to_path_buf()));
            self.dirs.borrow_mut().retain(|d| !d.starts_with(path));
            self.files.borrow_mut().retain(|p, _| !p.starts_with(path));
            Ok(())
        }
    }

    /// Scripted prompter that pops answers from VecDeques. Tests
    /// populate this up front; the install flow calls each prompt in
    /// order.
    pub struct MockPrompter {
        pub text: RefCell<std::collections::VecDeque<String>>,
        pub secret: RefCell<std::collections::VecDeque<SecretString>>,
        pub yn: RefCell<std::collections::VecDeque<bool>>,
        pub select: RefCell<std::collections::VecDeque<usize>>,
    }

    impl MockPrompter {
        pub fn new() -> Self {
            Self {
                text: RefCell::new(Default::default()),
                secret: RefCell::new(Default::default()),
                yn: RefCell::new(Default::default()),
                select: RefCell::new(Default::default()),
            }
        }

        pub fn push_text(&self, v: &str) -> &Self {
            self.text.borrow_mut().push_back(v.to_string());
            self
        }
        pub fn push_secret(&self, v: &str) -> &Self {
            self.secret
                .borrow_mut()
                .push_back(SecretString::new(v.to_string()));
            self
        }
        pub fn push_yn(&self, v: bool) -> &Self {
            self.yn.borrow_mut().push_back(v);
            self
        }
        pub fn push_select(&self, idx: usize) -> &Self {
            self.select.borrow_mut().push_back(idx);
            self
        }
    }

    impl Default for MockPrompter {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Prompter for MockPrompter {
        fn prompt_text(&self, message: &str, default: Option<&str>) -> Result<String> {
            self.text
                .borrow_mut()
                .pop_front()
                .or_else(|| default.map(|s| s.to_string()))
                .ok_or_else(|| anyhow!("MockPrompter: no text answer for `{message}`"))
        }
        fn prompt_secret(&self, message: &str) -> Result<SecretString> {
            self.secret
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow!("MockPrompter: no secret answer for `{message}`"))
        }
        fn prompt_yn(&self, message: &str, _default: bool) -> Result<bool> {
            self.yn
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow!("MockPrompter: no yn answer for `{message}`"))
        }
        fn prompt_select(&self, message: &str, _items: &[String]) -> Result<usize> {
            self.select
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow!("MockPrompter: no select answer for `{message}`"))
        }
    }
}
