use anyhow::{anyhow, Context, Result};
use secrecy::Secret;
use std::env;
use std::str::FromStr;

/// Driver for interactive prompts. Production uses `InquirePrompter`;
/// tests use `ScriptedPrompter` to feed canned responses.
pub trait Prompter {
    fn text(&self, message: &str, default: Option<&str>) -> Result<String>;
    fn secret(&self, message: &str, confirm: bool) -> Result<Secret<String>>;
    fn yes_no(&self, message: &str, default: bool) -> Result<bool>;
    fn select(&self, message: &str, items: &[String]) -> Result<usize>;
}

/// Resolve a value with the documented precedence:
///   1. `cli_value` (operator passed `--flag`)
///   2. env var (operator exported it)
///   3. interactive prompt (operator answers now)
///   4. `default` (only consulted when `--no-prompt` is set)
pub fn resolve<T, P>(
    env_var: &str,
    cli_value: Option<T>,
    default: Option<T>,
    no_prompt: bool,
    prompt_fn: P,
) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
    P: FnOnce() -> Result<T>,
{
    if let Some(v) = cli_value {
        return Ok(v);
    }
    if let Ok(raw) = env::var(env_var) {
        if !raw.is_empty() {
            return raw
                .parse::<T>()
                .map_err(|e| anyhow!("{env_var}={raw} is not parseable: {e}"));
        }
    }
    if no_prompt {
        if let Some(d) = default {
            return Ok(d);
        }
        return Err(anyhow!("missing required input (set {env_var} or pass the matching --flag); --no-prompt is on so we cannot ask interactively"));
    }
    prompt_fn().with_context(|| format!("prompt for {env_var}"))
}

/// Resolve a secret value (admin password, API key) with the same
/// precedence as `resolve`, but never echoes the value.
pub fn resolve_secret<P>(
    env_var: &str,
    cli_value: Option<String>,
    no_prompt: bool,
    prompt_fn: P,
) -> Result<Secret<String>>
where
    P: FnOnce() -> Result<Secret<String>>,
{
    if let Some(v) = cli_value {
        return Ok(Secret::new(v));
    }
    if let Ok(raw) = env::var(env_var) {
        if !raw.is_empty() {
            return Ok(Secret::new(raw));
        }
    }
    if no_prompt {
        return Err(anyhow!("missing required secret (set {env_var} or pass the matching --flag); --no-prompt is on so we cannot ask interactively"));
    }
    prompt_fn().with_context(|| format!("prompt for {env_var}"))
}

// ---------------------------------------------------------------------
// Inquire-backed prompter used in production.
// ---------------------------------------------------------------------

pub struct InquirePrompter;

impl Prompter for InquirePrompter {
    fn text(&self, message: &str, default: Option<&str>) -> Result<String> {
        let mut t = inquire::Text::new(message);
        if let Some(d) = default {
            t = t.with_default(d);
        }
        t.prompt()
            .map_err(|e| anyhow!("text prompt cancelled: {e}"))
    }

    fn secret(&self, message: &str, confirm: bool) -> Result<Secret<String>> {
        let mut p = inquire::Password::new(message);
        if !confirm {
            p = p.without_confirmation();
        }
        // `with_display_mode(Masked)` shows `*` while the operator
        // types, which is the usual UX even though it leaks length;
        // keep it for now.
        let s = p
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()
            .map_err(|e| anyhow!("password prompt cancelled: {e}"))?;
        Ok(Secret::new(s))
    }

    fn yes_no(&self, message: &str, default: bool) -> Result<bool> {
        inquire::Confirm::new(message)
            .with_default(default)
            .prompt()
            .map_err(|e| anyhow!("yes/no prompt cancelled: {e}"))
    }

    fn select(&self, message: &str, items: &[String]) -> Result<usize> {
        let options: Vec<String> = items.to_vec();
        let answer = inquire::Select::new(message, options.clone())
            .prompt()
            .map_err(|e| anyhow!("select prompt cancelled: {e}"))?;
        options
            .iter()
            .position(|s| s == &answer)
            .ok_or_else(|| anyhow!("selected option not in list"))
    }
}

// ---------------------------------------------------------------------
// Scripted prompter used in tests.
// ---------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct ScriptedAnswers {
    pub texts: Vec<String>,
    pub secrets: Vec<String>,
    pub yn: Vec<bool>,
    pub selects: Vec<usize>,
}

pub struct ScriptedPrompter {
    inner: std::cell::RefCell<ScriptedAnswers>,
}

impl ScriptedPrompter {
    pub fn new(answers: ScriptedAnswers) -> Self {
        Self {
            inner: std::cell::RefCell::new(answers),
        }
    }
}

impl Prompter for ScriptedPrompter {
    fn text(&self, _message: &str, default: Option<&str>) -> Result<String> {
        let mut a = self.inner.borrow_mut();
        if a.texts.is_empty() {
            if let Some(d) = default {
                return Ok(d.to_string());
            }
            return Err(anyhow!("ScriptedPrompter: out of text answers"));
        }
        Ok(a.texts.remove(0))
    }

    fn secret(&self, _message: &str, _confirm: bool) -> Result<Secret<String>> {
        let mut a = self.inner.borrow_mut();
        if a.secrets.is_empty() {
            return Err(anyhow!("ScriptedPrompter: out of secret answers"));
        }
        Ok(Secret::new(a.secrets.remove(0)))
    }

    fn yes_no(&self, _message: &str, default: bool) -> Result<bool> {
        let mut a = self.inner.borrow_mut();
        if a.yn.is_empty() {
            return Ok(default);
        }
        Ok(a.yn.remove(0))
    }

    fn select(&self, _message: &str, _items: &[String]) -> Result<usize> {
        let mut a = self.inner.borrow_mut();
        if a.selects.is_empty() {
            return Ok(0);
        }
        Ok(a.selects.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_wins() {
        // SAFETY: scoped to this test process; we explicitly clear.
        std::env::remove_var("COTERIE_PROVISION_TEST_CLIVAR");
        let v: String = resolve(
            "COTERIE_PROVISION_TEST_CLIVAR",
            Some("from-flag".to_string()),
            None,
            true,
            || Ok("from-prompt".to_string()),
        )
        .unwrap();
        assert_eq!(v, "from-flag");
    }

    #[test]
    fn env_wins_when_cli_absent() {
        std::env::set_var("COTERIE_PROVISION_TEST_ENVVAR", "from-env");
        let v: String =
            resolve::<String, _>("COTERIE_PROVISION_TEST_ENVVAR", None, None, true, || {
                Ok("from-prompt".to_string())
            })
            .unwrap();
        assert_eq!(v, "from-env");
        std::env::remove_var("COTERIE_PROVISION_TEST_ENVVAR");
    }

    #[test]
    fn no_prompt_uses_default_when_set() {
        std::env::remove_var("COTERIE_PROVISION_TEST_DEFVAR");
        let v: String = resolve(
            "COTERIE_PROVISION_TEST_DEFVAR",
            None,
            Some("the-default".to_string()),
            true,
            || Err(anyhow!("should not be called")),
        )
        .unwrap();
        assert_eq!(v, "the-default");
    }

    #[test]
    fn no_prompt_without_default_errors() {
        std::env::remove_var("COTERIE_PROVISION_TEST_FAILVAR");
        let r: Result<String> = resolve("COTERIE_PROVISION_TEST_FAILVAR", None, None, true, || {
            Ok("would-prompt".to_string())
        });
        assert!(r.is_err());
    }

    #[test]
    fn scripted_prompter_dispenses_in_order() {
        use secrecy::ExposeSecret;
        let p = ScriptedPrompter::new(ScriptedAnswers {
            texts: vec!["a".into(), "b".into()],
            secrets: vec!["password".into()],
            yn: vec![true, false],
            selects: vec![2],
        });
        assert_eq!(p.text("?", None).unwrap(), "a");
        assert_eq!(p.text("?", None).unwrap(), "b");
        assert_eq!(p.secret("?", true).unwrap().expose_secret(), "password");
        assert!(p.yes_no("?", false).unwrap());
        assert!(!p.yes_no("?", true).unwrap());
        assert_eq!(p.select("?", &[]).unwrap(), 2);
    }
}
