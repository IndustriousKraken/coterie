//! Smoke tests for clap argument parsing on the install subcommand.

use clap::Parser;
use coterie_provision::install::InstallArgs;

#[derive(Parser)]
#[command(name = "coterie-provision")]
struct Wrap {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Subcommand)]
enum Cmd {
    Install(InstallArgs),
}

#[test]
fn parses_minimal_invocation() {
    let cli = Wrap::try_parse_from(["coterie-provision", "install"]).expect("parse");
    let Cmd::Install(args) = cli.cmd;
    assert!(!args.no_prompt);
    assert!(!args.dry_run);
    assert!(args.org_name.is_none());
}

#[test]
fn parses_all_flags() {
    let cli = Wrap::try_parse_from([
        "coterie-provision",
        "install",
        "--org-name",
        "ACME",
        "--portal-domain",
        "coterie.example.com",
        "--marketing-domain",
        "example.com",
        "--contact-email",
        "ops@example.com",
        "--admin-email",
        "admin@example.com",
        "--admin-username",
        "admin",
        "--admin-full-name",
        "Admin User",
        "--admin-password",
        "p@ss",
        "--enable-stripe",
        "true",
        "--enable-caddy",
        "false",
        "--enable-discord",
        "true",
        "--enable-unifi",
        "false",
        "--stripe-publishable-key",
        "pk_test_x",
        "--stripe-secret-key",
        "sk_test_x",
        "--stripe-webhook-secret",
        "whsec_x",
        "--version",
        "v1.0.0",
        "--no-prompt",
        "--dry-run",
    ])
    .expect("parse");
    let Cmd::Install(args) = cli.cmd;
    assert_eq!(args.org_name.as_deref(), Some("ACME"));
    assert_eq!(args.portal_domain.as_deref(), Some("coterie.example.com"));
    assert_eq!(args.marketing_domain.as_deref(), Some("example.com"));
    assert_eq!(args.enable_stripe, Some(true));
    assert_eq!(args.enable_caddy, Some(false));
    assert_eq!(args.enable_discord, Some(true));
    assert_eq!(args.enable_unifi, Some(false));
    assert_eq!(args.version.as_deref(), Some("v1.0.0"));
    assert!(args.no_prompt);
    assert!(args.dry_run);
}

#[test]
fn boolean_parses_false_string() {
    let cli = Wrap::try_parse_from(["coterie-provision", "install", "--enable-stripe", "false"])
        .expect("parse");
    let Cmd::Install(args) = cli.cmd;
    assert_eq!(args.enable_stripe, Some(false));
}
