use anyhow::{anyhow, Context, Result};
use secrecy::SecretString;
use std::str::FromStr;

/// Resolves an input value via the chain: cli flag → env var →
/// (interactive prompt OR default if non-interactive).
///
/// Errors when running non-interactive (`no_prompt = true`) with no
/// flag, no env var, and no default — listing the source the operator
/// should set.
pub fn resolve<T, F>(
    name: &str,
    env_var: &str,
    cli_value: Option<T>,
    default: Option<T>,
    no_prompt: bool,
    prompt_fn: F,
) -> Result<T>
where
    T: FromStr + Clone,
    T::Err: std::fmt::Display,
    F: FnOnce() -> Result<T>,
{
    if let Some(v) = cli_value {
        return Ok(v);
    }
    if let Ok(raw) = std::env::var(env_var) {
        if !raw.is_empty() {
            return raw
                .parse::<T>()
                .map_err(|e| anyhow!("env var {env_var} could not be parsed as {name}: {e}"));
        }
    }
    if no_prompt {
        if let Some(d) = default {
            return Ok(d);
        }
        return Err(anyhow!(
            "missing required input `{name}` — set the {env_var} env var or pass --{name} (running with --no-prompt so cannot ask interactively)"
        ));
    }
    prompt_fn().with_context(|| format!("failed to read `{name}` interactively"))
}

/// A test-friendly prompter trait. Production uses `InquirePrompter`
/// which delegates to the `inquire` crate; tests use a `MockPrompter`
/// that scripts every prompt up front.
pub trait Prompter {
    fn prompt_text(&self, message: &str, default: Option<&str>) -> Result<String>;
    fn prompt_secret(&self, message: &str) -> Result<SecretString>;
    fn prompt_yn(&self, message: &str, default: bool) -> Result<bool>;
    fn prompt_select(&self, message: &str, items: &[String]) -> Result<usize>;
}

pub struct InquirePrompter;

impl InquirePrompter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for InquirePrompter {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompter for InquirePrompter {
    fn prompt_text(&self, message: &str, default: Option<&str>) -> Result<String> {
        let mut p = inquire::Text::new(message);
        if let Some(d) = default {
            p = p.with_default(d);
        }
        p.prompt().context("text prompt failed")
    }

    fn prompt_secret(&self, message: &str) -> Result<SecretString> {
        let value = inquire::Password::new(message)
            .with_display_toggle_enabled()
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()
            .context("password prompt failed")?;
        Ok(SecretString::new(value))
    }

    fn prompt_yn(&self, message: &str, default: bool) -> Result<bool> {
        inquire::Confirm::new(message)
            .with_default(default)
            .prompt()
            .context("yes/no prompt failed")
    }

    fn prompt_select(&self, message: &str, items: &[String]) -> Result<usize> {
        let chosen = inquire::Select::new(message, items.to_vec())
            .prompt()
            .context("select prompt failed")?;
        items
            .iter()
            .position(|s| s == &chosen)
            .ok_or_else(|| anyhow!("internal: selected item not in list"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct ScriptedPrompter {
        text_answers: RefCell<Vec<String>>,
    }

    impl Prompter for ScriptedPrompter {
        fn prompt_text(&self, _message: &str, _default: Option<&str>) -> Result<String> {
            self.text_answers
                .borrow_mut()
                .pop()
                .ok_or_else(|| anyhow!("no scripted answer"))
        }
        fn prompt_secret(&self, _: &str) -> Result<SecretString> {
            unimplemented!()
        }
        fn prompt_yn(&self, _: &str, _: bool) -> Result<bool> {
            unimplemented!()
        }
        fn prompt_select(&self, _: &str, _: &[String]) -> Result<usize> {
            unimplemented!()
        }
    }

    #[test]
    fn cli_wins_over_env() {
        std::env::set_var("FOO_VAR", "env-val");
        let p = ScriptedPrompter {
            text_answers: RefCell::new(vec![]),
        };
        let v: String = resolve(
            "foo",
            "FOO_VAR",
            Some("cli-val".to_string()),
            None,
            false,
            || p.prompt_text("m", None),
        )
        .unwrap();
        assert_eq!(v, "cli-val");
        std::env::remove_var("FOO_VAR");
    }

    #[test]
    fn env_used_when_no_cli() {
        std::env::set_var("BAR_VAR", "env-val");
        let p = ScriptedPrompter {
            text_answers: RefCell::new(vec![]),
        };
        let v: String = resolve("bar", "BAR_VAR", None, None, false, || {
            p.prompt_text("m", None)
        })
        .unwrap();
        assert_eq!(v, "env-val");
        std::env::remove_var("BAR_VAR");
    }

    #[test]
    fn no_prompt_errors_without_default() {
        std::env::remove_var("BAZ_VAR");
        let p = ScriptedPrompter {
            text_answers: RefCell::new(vec![]),
        };
        let err = resolve::<String, _>("baz", "BAZ_VAR", None, None, true, || {
            p.prompt_text("m", None)
        })
        .unwrap_err();
        assert!(err.to_string().contains("BAZ_VAR"));
    }

    #[test]
    fn no_prompt_uses_default() {
        std::env::remove_var("QUX_VAR");
        let p = ScriptedPrompter {
            text_answers: RefCell::new(vec![]),
        };
        let v: String = resolve(
            "qux",
            "QUX_VAR",
            None,
            Some("def".to_string()),
            true,
            || p.prompt_text("m", None),
        )
        .unwrap();
        assert_eq!(v, "def");
    }

    #[test]
    fn falls_back_to_prompt() {
        std::env::remove_var("ZOT_VAR");
        let p = ScriptedPrompter {
            text_answers: RefCell::new(vec!["typed-val".to_string()]),
        };
        let v: String = resolve("zot", "ZOT_VAR", None, None, false, || {
            p.prompt_text("m", None)
        })
        .unwrap();
        assert_eq!(v, "typed-val");
    }
}
