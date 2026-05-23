use std::cell::RefCell;

/// Stdout abstraction. Production writes through `println!`; tests
/// capture each line for assertion.
pub trait Output {
    fn println(&self, line: &str);
}

pub struct RealOutput;

impl RealOutput {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Output for RealOutput {
    fn println(&self, line: &str) {
        println!("{line}");
    }
}

/// Captures every printed line for assertion in tests.
pub struct CaptureOutput {
    pub lines: RefCell<Vec<String>>,
}

impl CaptureOutput {
    pub fn new() -> Self {
        Self {
            lines: RefCell::new(Vec::new()),
        }
    }

    pub fn joined(&self) -> String {
        self.lines.borrow().join("\n")
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.joined().contains(needle)
    }
}

impl Default for CaptureOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Output for CaptureOutput {
    fn println(&self, line: &str) {
        self.lines.borrow_mut().push(line.to_string());
    }
}
