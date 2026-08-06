use std::io::{self, BufRead, Write};

#[path = "../../git_remote_common/mod.rs"]
mod common;

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let response = common::dispatch_line("git_forge", &line?, common::forge::execute_git_forge);
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}
