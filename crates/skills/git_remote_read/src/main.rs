use std::io::{self, BufRead, Write};

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let response = git_remote_common::dispatch_line(
            "git_remote_read",
            &line?,
            git_remote_common::git::execute_remote_read,
        );
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}
