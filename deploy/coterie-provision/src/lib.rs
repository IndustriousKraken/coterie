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
    use std::collections::{HashMap, HashSet, VecDeque};
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
    ///
    /// Lookup precedence on each call:
    /// 1. If `sequenced_responses[(cmd, args)]` is non-empty, pop the
    ///    front `CommandOutput`. This lets tests script "first 2 fail,
    ///    3rd succeeds" without manually re-registering between calls.
    /// 2. Else if `responses[(cmd, args)]` is set, clone it.
    /// 3. Else if `responses_by_cmd[cmd]` is set, clone it. (Useful
    ///    when args contain a dynamic value like a tempfile path.)
    /// 4. Else `default_response`.
    pub struct FakeSystem {
        pub calls: RefCell<Vec<RecordedCall>>,
        pub responses: RefCell<HashMap<CommandKey, CommandOutput>>,
        pub sequenced_responses: RefCell<HashMap<CommandKey, VecDeque<CommandOutput>>>,
        pub responses_by_cmd: RefCell<HashMap<String, CommandOutput>>,
        pub default_response: CommandOutput,
    }

    impl FakeSystem {
        pub fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(HashMap::new()),
                sequenced_responses: RefCell::new(HashMap::new()),
                responses_by_cmd: RefCell::new(HashMap::new()),
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

        /// Queue a sequence of responses for repeated calls to the same
        /// command. Each call pops one from the front. Useful for
        /// modeling retry-then-succeed (smoke-test polling).
        pub fn respond_to_sequence(&self, cmd: &str, args: &[&str], outs: Vec<CommandOutput>) {
            self.sequenced_responses
                .borrow_mut()
                .insert(CommandKey::new(cmd, args), outs.into_iter().collect());
        }

        /// Respond to any invocation of `cmd`, regardless of args.
        /// Useful when args carry a dynamic value (tempfile path, etc.)
        /// that makes exact-key matching impractical.
        pub fn respond_to_cmd(&self, cmd: &str, out: CommandOutput) {
            self.responses_by_cmd
                .borrow_mut()
                .insert(cmd.to_string(), out);
        }

        fn lookup_response(&self, key: &CommandKey) -> CommandOutput {
            if let Some(queue) = self.sequenced_responses.borrow_mut().get_mut(key) {
                if let Some(out) = queue.pop_front() {
                    return out;
                }
            }
            if let Some(out) = self.responses.borrow().get(key) {
                return out.clone();
            }
            if let Some(out) = self.responses_by_cmd.borrow().get(&key.cmd) {
                return out.clone();
            }
            self.default_response.clone()
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
            Ok(self.lookup_response(&CommandKey::new(cmd, args)))
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
            Ok(self.lookup_response(&CommandKey::new(cmd, args)))
        }

        fn run_interactive(&self, cmd: &str, args: &[&str]) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(RecordedCall {
                cmd: cmd.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                stdin: None,
                interactive: true,
            });
            Ok(self.lookup_response(&CommandKey::new(cmd, args)))
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
        /// Paths for which `chown` SHALL return Err. Tests use this to
        /// exercise the wizard's error-propagation paths.
        pub chown_failures: RefCell<HashSet<PathBuf>>,
    }

    impl FakeFs {
        pub fn new() -> Self {
            Self {
                files: RefCell::new(HashMap::new()),
                dirs: RefCell::new(Vec::new()),
                ops: RefCell::new(Vec::new()),
                chown_failures: RefCell::new(HashSet::new()),
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

        /// Configure `chown` to return Err whenever invoked on `path`.
        pub fn fail_chown_on(&self, path: &Path) {
            self.chown_failures.borrow_mut().insert(path.to_path_buf());
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
            if self.chown_failures.borrow().contains(path) {
                return Err(anyhow!(
                    "FakeFs: chown {user}:{group} {} configured to fail",
                    path.display()
                ));
            }
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
