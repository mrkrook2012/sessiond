# sessiond

A daemon and terminal manager. Run commands in persistent terminal sessions that keep
going after you close your shell, attach to them from a TUI, and turn any command into a
systemd user service in one line.

## Quick start: one-shot a systemd service

```sh
sessiond run --sysd --name webserver "python3 -m http.server 8080"
```

That's it. This:

1. starts the daemon in the background if it isn't running yet,
2. writes `~/.config/systemd/user/webserver.service`,
3. starts the unit through systemd, which spawns the session running your command.

Check on it and manage it like any other service:

```sh
sessiond list --services              # - webserver [service, running, disabled]
sessiond service enable webserver     # start it at boot
systemctl --user status webserver        # plain systemctl works too
```

If the command exits with an error, systemd restarts it (`Restart=on-failure`).

## Installation

From source:

```sh
cargo install --path .
```

Or grab the `.deb` from the [GitHub releases](https://github.com/mrkrook2012/sessiond2/releases)
and install it with `sudo apt install ./sessiond_*.deb`.

## Commands

Running `sessiond` with no subcommand opens the TUI, where you can browse and attach to
sessions.

### Sessions

| Command                               | Description                                                                                                                                                                        |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run [--sysd] [-n NAME] <command>...` | Start a new session running `<command>`. With `--sysd` (alias `--systemd`) it becomes a systemd service instead of an ephemeral session. Without `--name` it gets a random name like `banana-harbor`, which is printed. |
| `list [--names] [--services]`         | List active sessions. `--names` prints names only, one per line; `--services` shows only systemd-backed sessions.                                                                  |
| `attach <name>`                       | Open the TUI on just that session. Detaching (`Ctrl-b d`) or the session ending exits back to your shell. |
| `kill <name>`                         | Kill a session.                                                                                                                                                                    |
| `kill --all`                          | Kill every active session.                                                                                                                                                         |

`run` takes either a single shell command line or a command with arguments:

```sh
sessiond run "make && ./serve"       # one argument: interpreted by the shell
sessiond run sh -c 'sleep 300'       # several arguments: passed through exactly
```

Put `--sysd` and `--name` before the command: anything after the command's first word is
passed to the command.

Session names are matched case-insensitively, and a typo gets you a "Did you mean ...?"
suggestion.

### Services (systemd-backed sessions)

| Command                  | Description                                                                                             |
| ------------------------ | ------------------------------------------------------------------------------------------------------- |
| `service start <name>`   | Start the service.                                                                                      |
| `service stop <name>`    | Stop the service.                                                                                       |
| `service restart <name>` | Restart the service.                                                                                    |
| `service enable <name>`  | Start the service at boot.                                                                              |
| `service disable <name>` | Don't start the service at boot.                                                                        |
| `service update <name>`  | Save the running service's current windows to its session file, so the next (re)start brings them back. |
| `service delete <name>`  | Delete the service and its unit/session files.                                                          |

Changes made with `systemctl --user` (start, stop, enable, disable) are picked up by the
daemon as well.

### Export / import (ephemeral sessions)

| Command                  | Description                                                                   |
| ------------------------ | ----------------------------------------------------------------------------- |
| `export <name> [-d DIR]` | Export a session to a YAML config file in `DIR` (default: current directory). |
| `import <file>`          | Start a new session from an exported config file.                             |

### Daemon

| Command         | Description                                                                                  |
| --------------- | -------------------------------------------------------------------------------------------- |
| `daemon start`  | Run the daemon in the foreground. Usually not needed: other commands start it automatically. |
| `daemon stop`   | Stop the daemon and wait until it has exited.                                                |
| `daemon status` | Exit with code 0 if the daemon is running, 1 otherwise.                                      |

### Shell completions

```sh
sessiond completions fish > ~/.config/fish/completions/sessiond.fish
```

Completions include live session names.

### Global flags

| Flag                  | Description                        |
| --------------------- | ---------------------------------- |
| `-v` / `-vv` / `-vvv` | Log at info / debug / trace level. |
| `-q`                  | Log errors only.                   |

`RUST_LOG` overrides these.

CLI commands log to stderr; under systemd that ends up in the journal
(`journalctl --user -u <name>`). The daemon and the TUI log to files in the log directory
(see below). When a process starts, it moves the previous log to `<name>.log.old`.

## Files and environment

| Path / variable                         | Purpose                                                                                       |
| --------------------------------------- | --------------------------------------------------------------------------------------------- |
| `/tmp/sessiond.sock`                    | Daemon socket. Override with `SESSIOND_SOCKET`.                                               |
| `~/.config/systemd/user/<name>.service` | Generated unit files.                                                                         |
| `~/.local/share/sessiond/<name>.yaml`   | Session files for services.                                                                   |
| `~/.local/state/sessiond/daemon.log`   | Log of the background daemon (stdout, stderr and panics).                                     |
| `~/.local/state/sessiond/frontend.log` | Log of the TUI.                                                                               |
| `SESSIOND_LOG_DIR`                      | Overrides the log directory (default: `$XDG_STATE_HOME/sessiond`, else `~/.local/state/sessiond`). |

## Development

```sh
cargo test --all-targets
```

CI runs the tests on every push, builds a `.deb` with `cargo deb`, and publishes a GitHub
release for `v*` tags.

## License

MIT

---

<sub>
Killer of the night,<br>
Always sees you,<br>
Hiding in the green
</sub>
