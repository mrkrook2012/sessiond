
# ---------------------------------------------------------------------------
# Hand-written part: things clap cannot know about.
# ---------------------------------------------------------------------------

# True when the non-option words after `daemon_test` are exactly $argv, i.e. the
# cursor is at the first positional argument of that (sub)command.
# Example: `__daemon_test_at_positional service start` matches `daemon_test service start <TAB>`
# but not `daemon_test service start foo <TAB>`.
function __daemon_test_at_positional
    set -l tokens (commandline -opc)
    set -e tokens[1]

    set -l words
    set -l skip_value 0
    for token in $tokens
        if test $skip_value -eq 1
            set skip_value 0
            continue
        end
        switch $token
            # Options that take a value: the value is not a positional.
            case -n --name -d --dir
                set skip_value 1
            case '-*'
            case '*'
                set -a words $token
        end
    end

    test "$words" = "$argv"
end

# Session names from the running daemon. Prints nothing if the daemon is not running,
# so completion never starts the daemon. Extra arguments go to `list`.
function __daemon_test_sessions
    command daemon_test list --names $argv 2>/dev/null
end

# None of these take files (`export --dir` brings its own directory completion).
complete -c daemon_test -n "__fish_daemon_test_using_subcommand attach kill export service list daemon completions" -f

# Commands that take any session.
for cmd in attach kill export
    complete -c daemon_test -n "__daemon_test_at_positional $cmd" -f -a "(__daemon_test_sessions)"
end

# Commands that take a systemd session.
for action in start stop restart enable disable delete
    complete -c daemon_test -n "__daemon_test_at_positional service $action" -f -a "(__daemon_test_sessions --services)"
end

# `run [options] <command> [args...]`: complete the wrapped command like `sudo` does.
# Skips `daemon_test` and `run`, and the value of -n/--name.
complete -c daemon_test -n "__fish_daemon_test_using_subcommand run" -f -a "(__fish_complete_subcommand --fcs-skip=2 -n --name)"

complete -c daemon_test -n "__daemon_test_at_positional completions" -f -a "fish" -d "fish shell"
