use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalCapabilities {
    pub(crate) stdin_tty: bool,
    pub(crate) stdout_tty: bool,
    pub(crate) color: bool,
    pub(crate) animation: bool,
    pub(crate) width: Option<usize>,
}

pub(crate) fn detect(machine_mode: bool) -> TerminalCapabilities {
    detect_with(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        machine_mode,
        std::env::var_os("NO_COLOR").is_some(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var("COLUMNS").ok().as_deref(),
    )
}

fn detect_with(
    stdin_tty: bool,
    stdout_tty: bool,
    machine_mode: bool,
    no_color: bool,
    term: Option<&str>,
    columns: Option<&str>,
) -> TerminalCapabilities {
    let minimal_term = term.is_some_and(|term| term.eq_ignore_ascii_case("dumb"));
    let human_tty = stdin_tty && stdout_tty && !machine_mode && !minimal_term;
    TerminalCapabilities {
        stdin_tty,
        stdout_tty,
        color: human_tty && !no_color,
        animation: human_tty && !no_color,
        width: columns
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|value| (20..=1000).contains(value)),
    }
}

#[cfg(test)]
#[path = "terminal_capabilities_tests.rs"]
mod tests;
