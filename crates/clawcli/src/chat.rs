use anyhow::{Context, Result};

use crate::task;

const POLL_INTERVAL_MS: u64 = 800;
const TERMINAL_STATUS: &[&str] = &["succeeded", "failed", "canceled"];

pub(crate) fn run_chat(base_url: &str, key: &str) -> Result<()> {
    println!("clawcli chat mode (type a message, empty line or 'exit' to quit)");
    println!("---");
    let mut rl = rustyline::DefaultEditor::new().context("rustyline init (is stdin a TTY?)")?;
    loop {
        let line = match rl.readline("> ") {
            Ok(s) => s,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => break,
            Err(e) => {
                eprintln!("readline: {}", e);
                break;
            }
        };
        let text = line.trim();
        if text.is_empty() {
            break;
        }
        if text.eq_ignore_ascii_case("exit") || text.eq_ignore_ascii_case("quit") {
            break;
        }
        let task_id = match task::submit_ask(base_url, key, text) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("submit failed: {}", e);
                continue;
            }
        };
        let mut wait_tick = 0usize;
        loop {
            let dots = match wait_tick % 4 {
                0 => ".",
                1 => "..",
                2 => "...",
                _ => "",
            };
            print!("\rWaiting for clawd reply{dots:<3}");
            std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
            wait_tick += 1;
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            let task = match task::get_task_status(base_url, key, &task_id) {
                Ok(t) => t,
                Err(e) => {
                    print!("\r{:<48}\r", "");
                    std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
                    eprintln!("get task failed: {}", e);
                    break;
                }
            };
            if TERMINAL_STATUS.contains(&task.status.as_str()) {
                print!("\r{:<48}\r", "");
                std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
                if let Some(ref t) = task.result_text {
                    println!("{}\n", t);
                }
                if let Some(ref e) = task.error_text {
                    eprintln!("error: {}\n", e);
                }
                if task.status == "failed"
                    && task.result_text.is_none()
                    && task.error_text.is_none()
                {
                    println!("task_failed_without_details\n");
                }
                break;
            }
        }
    }
    Ok(())
}
