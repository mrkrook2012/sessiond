//! Argument parsing and the pure helpers of the CLI. Behaviour against a daemon is in `tests/e2e`.

use std::fs;

use clap::{CommandFactory, Parser, error::ErrorKind};
use sessiond::cli::{
    Cli, Commands, DaemonCommands, ServiceCommands, command_line, generate_random_name, session_name_from_config, session_not_found_message,
    shell_join, shell_quote,
};
use tracing::level_filters::LevelFilter;

fn parse(args: &[&str]) -> Result<Cli, ErrorKind> {
    Cli::try_parse_from(std::iter::once("sessiond").chain(args.iter().copied()))
        .map_err(|e| e.kind())
}

fn command(args: &[&str]) -> Commands {
    match parse(args) {
        Ok(cli) => cli.command.expect("expected a subcommand"),
        Err(kind) => panic!("{args:?} did not parse: {kind:?}"),
    }
}

// ------------------------------------------------------------------ definition

#[test]
fn clap_definition_is_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn no_subcommand_starts_the_tui() {
    assert!(parse(&[]).unwrap().command.is_none());
}

// ------------------------------------------------------------------ run

#[test]
fn run_takes_the_rest_of_the_line_as_the_command() {
    let Commands::Run { sysd, name, command } = command(&["run", "-n", "x", "ls", "-la", "--color"])
    else {
        panic!("not run");
    };
    assert!(!sysd);
    assert_eq!(name.as_deref(), Some("x"));
    assert_eq!(command, vec!["ls", "-la", "--color"]);
}

#[test]
fn run_flags_after_the_command_belong_to_the_command() {
    let Commands::Run { sysd, name, command } = command(&["run", "echo", "--sysd", "-n", "y"])
    else {
        panic!("not run");
    };
    assert!(!sysd);
    assert_eq!(name, None);
    assert_eq!(command, vec!["echo", "--sysd", "-n", "y"]);
}

#[test]
fn run_sysd_flag() {
    let Commands::Run { sysd, .. } = command(&["run", "--sysd", "--name", "svc", "sleep", "1"])
    else {
        panic!("not run");
    };
    assert!(sysd);
}

#[test]
fn run_accepts_old_systemd_flag() {
    let Commands::Run { sysd, .. } = command(&["run", "--systemd", "sleep", "1"]) else {
        panic!("not run");
    };
    assert!(sysd);
}

#[test]
fn run_requires_a_command() {
    assert_eq!(parse(&["run"]).err(), Some(ErrorKind::MissingRequiredArgument));
    assert_eq!(parse(&["run", "-n", "x"]).err(), Some(ErrorKind::MissingRequiredArgument));
}

// ------------------------------------------------------------------ kill

#[test]
fn kill_takes_a_name_or_all_but_not_both() {
    assert!(matches!(command(&["kill", "a"]), Commands::Kill { name: Some(n), all: false } if n == "a"));
    assert!(matches!(command(&["kill", "--all"]), Commands::Kill { name: None, all: true }));
    assert_eq!(parse(&["kill"]).err(), Some(ErrorKind::MissingRequiredArgument));
    assert_eq!(parse(&["kill", "a", "--all"]).err(), Some(ErrorKind::ArgumentConflict));
}

// ------------------------------------------------------------------ other commands

#[test]
fn list_flags() {
    assert!(matches!(command(&["list"]), Commands::List { names: false, services: false }));
    assert!(matches!(
        command(&["list", "--names", "--services"]),
        Commands::List { names: true, services: true }
    ));
}

#[test]
fn service_subcommands() {
    let cases: [(&str, fn(&ServiceCommands) -> bool); 6] = [
        ("start", |c| matches!(c, ServiceCommands::Start { name } if name == "s")),
        ("stop", |c| matches!(c, ServiceCommands::Stop { name } if name == "s")),
        ("restart", |c| matches!(c, ServiceCommands::Restart { name } if name == "s")),
        ("enable", |c| matches!(c, ServiceCommands::Enable { name } if name == "s")),
        ("disable", |c| matches!(c, ServiceCommands::Disable { name } if name == "s")),
        ("delete", |c| matches!(c, ServiceCommands::Delete { name } if name == "s")),
    ];
    for (action, check) in cases {
        let Commands::Service { action: parsed } = command(&["service", action, "s"]) else {
            panic!("not service");
        };
        assert!(check(&parsed), "service {action} parsed wrong");
        assert!(parse(&["service", action]).is_err(), "service {action} needs a name");
    }
}

#[test]
fn daemon_subcommands() {
    assert!(matches!(command(&["daemon", "start"]), Commands::Daemon { action: DaemonCommands::Start }));
    assert!(matches!(command(&["daemon", "stop"]), Commands::Daemon { action: DaemonCommands::Stop }));
    assert!(matches!(command(&["daemon", "status"]), Commands::Daemon { action: DaemonCommands::Status }));
    assert!(parse(&["daemon"]).is_err());
}

#[test]
fn export_and_import_arguments() {
    let Commands::Export { name, dir } = command(&["export", "s", "-d", "/tmp"]) else {
        panic!("not export");
    };
    assert_eq!((name.as_str(), dir), ("s", Some("/tmp".into())));
    assert!(matches!(command(&["export", "s"]), Commands::Export { dir: None, .. }));
    assert!(matches!(command(&["import", "f.yaml"]), Commands::Import { file } if file.as_os_str() == "f.yaml"));
}

#[test]
fn hidden_internal_commands_parse_but_are_not_in_help() {
    assert!(matches!(
        command(&["__internal-sysd-start", "/x.yaml"]),
        Commands::InternalSysdStart { config } if config.as_os_str() == "/x.yaml"
    ));
    assert!(matches!(
        command(&["__internal-sysd-stop", "svc"]),
        Commands::InternalSysdStop { name } if name == "svc"
    ));

    let help = Cli::command().render_help().to_string();
    assert!(!help.contains("__internal"), "help shows internal commands:\n{help}");
}

#[test]
fn old_systemd_internal_command_names_still_parse() {
    // Unit files written before the rename still call these.
    assert!(matches!(
        command(&["__internal-systemd-start", "/x.yaml"]),
        Commands::InternalSysdStart { config } if config.as_os_str() == "/x.yaml"
    ));
    assert!(matches!(
        command(&["__internal-systemd-stop", "svc"]),
        Commands::InternalSysdStop { name } if name == "svc"
    ));
}

#[test]
fn unknown_subcommand_is_rejected() {
    assert_eq!(parse(&["frobnicate"]).err(), Some(ErrorKind::InvalidSubcommand));
}

#[test]
fn verbosity_flags_map_to_levels() {
    let level = |args: &[&str]| parse(args).unwrap().verbose.tracing_level_filter();
    assert_eq!(level(&["list"]), LevelFilter::WARN);
    assert_eq!(level(&["-v", "list"]), LevelFilter::INFO);
    assert_eq!(level(&["-vv", "list"]), LevelFilter::DEBUG);
    assert_eq!(level(&["-vvv", "list"]), LevelFilter::TRACE);
    assert_eq!(level(&["-q", "list"]), LevelFilter::ERROR);
    assert_eq!(level(&["-qq", "list"]), LevelFilter::OFF);
    // Global flag: also accepted after the subcommand.
    assert_eq!(level(&["list", "-v"]), LevelFilter::INFO);
}

// ------------------------------------------------------------------ helpers

#[test]
fn shell_quote_leaves_safe_words_alone() {
    for word in ["ls", "-la", "--color=auto", "/usr/bin/env", "a.b,c:d@e%f+g_h"] {
        assert_eq!(shell_quote(word), word);
    }
}

#[test]
fn shell_quote_quotes_everything_else() {
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("a b"), "'a b'");
    assert_eq!(shell_quote("$HOME"), "'$HOME'");
    assert_eq!(shell_quote("a;b"), "'a;b'");
    assert_eq!(shell_quote("it's"), r"'it'\''s'");
}

#[test]
fn shell_join_round_trips_through_a_shell() {
    let args: Vec<String> = ["printf", "%s|", "a b", "it's", "", "$HOME", "*"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let line = shell_join(&args);

    let out = std::process::Command::new("sh").args(["-c", &line]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|it's||$HOME|*|");
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_single_argument_is_a_command_line_and_runs_as_one() {
    let line = command_line(&strings(&["echo hello && echo world"]));
    assert_eq!(line, "echo hello && echo world");

    let out = std::process::Command::new("sh").args(["-c", &line]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "hello\nworld\n");
}

#[test]
fn several_arguments_are_passed_on_exactly() {
    assert_eq!(command_line(&strings(&["sh", "-c", "sleep 300"])), "sh -c 'sleep 300'");
    // Shell syntax in one of several arguments stays literal.
    assert_eq!(command_line(&strings(&["echo", "a && b"])), "echo 'a && b'");
}

#[test]
fn generated_names_come_from_the_fixed_list() {
    for _ in 0..50 {
        let name = generate_random_name();
        assert!(
            ["henk", "bink", "aardappel", "piet", "jannes"].contains(&name.as_str()),
            "unexpected name {name}"
        );
    }
}

#[test]
fn session_name_from_config_is_the_lowercased_unit_name() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("s.yaml");
    fs::write(&path, "name: MySvc\nwindows: []\n").unwrap();
    assert_eq!(session_name_from_config(&path).unwrap(), "mysvc");

    fs::write(&path, "not yaml: [").unwrap();
    assert!(session_name_from_config(&path).is_err());
    assert!(session_name_from_config(&dir.path().join("missing.yaml")).is_err());
}

// ------------------------------------------------------------------ session not found

#[test]
fn session_not_found_points_to_list_without_suggestions() {
    let message = session_not_found_message("zzz", &[]);
    assert_eq!(message, "Session not found: 'zzz'\nRun `sessiond list` to see all sessions.");
}

#[test]
fn session_not_found_suggests_one_name() {
    let message = session_not_found_message("websrever", &["webserver".into()]);
    assert_eq!(message, "Session not found: 'websrever'\nDid you mean 'webserver'?");
}

#[test]
fn session_not_found_lists_several_names() {
    let similar = ["web-proxy".into(), "webserver".into(), "web".into()];
    let message = session_not_found_message("wb", &similar);
    assert_eq!(
        message,
        "Session not found: 'wb'\nDid you mean 'web-proxy', 'webserver' or 'web'?"
    );
}
