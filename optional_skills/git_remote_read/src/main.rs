use std::io::{self, BufRead, Write};

#[path = "../../git_remote_common/mod.rs"]
mod common;

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let response =
            common::dispatch_line("git_remote_read", &line?, common::git::execute_remote_read);
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}
