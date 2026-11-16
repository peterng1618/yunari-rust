//! The stdio bridge to a running host's MCP server.
//!
//! An agent that speaks JSON-RPC on stdin gets one line back per request. The
//! host does the answering; this program only finds the port it is on, so a
//! client never needs to know it.
//!
//! `install`, `uninstall` and `status` manage that client configuration and
//! the theme-designer skill; with no argument the bridge is what runs.

use std::io::{BufRead, Write};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some(command @ ("install" | "uninstall" | "status")) => {
            std::process::exit(manage(command.to_owned()));
        }
        Some("help" | "--help" | "-h") => {
            println!(
                "yunari-mcp [install|uninstall|status]\n\
                 \n\
                 With no argument, bridges stdin to the running Yunari host's MCP server.\n\
                 install    registers this bridge with Claude Code and copies the theme-designer skill\n\
                 uninstall  removes both\n\
                 status     reports both, exiting 0 only when the bridge and skill are installed"
            );
        }
        _ => bridge(),
    }
}

/// Runs one of the managing subcommands, printing its report as one JSON line.
fn manage(command: String) -> i32 {
    let Some(home) = yunari_host::mcp_install::home() else {
        eprintln!("No user profile is set, so there is nowhere to install.");
        return 2;
    };
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            eprintln!("Locating this program failed: {error}");
            return 2;
        }
    };
    let report = match command.as_str() {
        "install" => yunari_host::mcp_install::install(&home, &executable),
        "uninstall" => yunari_host::mcp_install::uninstall(&home),
        _ => yunari_host::mcp_install::status(&home),
    };
    println!("{}", report.json());
    if let Some(message) = &report.message {
        eprintln!("{message}");
        return 1;
    }
    let ready = report.server && report.skill == yunari_host::mcp_install::SkillState::Current;
    match command.as_str() {
        "status" if !ready => 1,
        _ => 0,
    }
}

fn bridge() {
    let config = yunari_host::config::HostConfig::resolve(None, None, false);
    let settings = config.settings_dir();
    let port = match locked_port(&settings) {
        Some(port) => port,
        None => {
            eprintln!(
                "No Yunari host is running over {}. Start the app first.",
                settings.display()
            );
            std::process::exit(1);
        }
    };
    let url = format!("http://127.0.0.1:{port}/api/mcp");
    let agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let reply = match agent
            .post(&url)
            .header("content-type", "application/json")
            .send(line)
        {
            Ok(mut response) => {
                let status = response.status().as_u16();
                let body = response.body_mut().read_to_string().unwrap_or_default();
                if (200..300).contains(&status) && !body.trim().is_empty() {
                    body
                } else {
                    error_line(status, &body)
                }
            }
            Err(error) => error_line(0, &error.to_string()),
        };
        if writeln!(stdout, "{reply}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
}

/// The port the host recorded when it took the settings folder.
fn locked_port(settings: &std::path::Path) -> Option<u16> {
    let text = std::fs::read_to_string(settings.join("host.lock")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("port")?.as_u64()?.try_into().ok()
}

/// An answer in the protocol's own error shape, so a client sees a refusal
/// rather than a closed pipe.
fn error_line(status: u16, body: &str) -> String {
    let reason = if body.trim().is_empty() {
        format!("the host did not answer (status {status})")
    } else {
        body.trim().to_owned()
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": {"code": -32603, "message": reason},
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{error_line, locked_port};

    #[test]
    fn the_port_comes_from_the_host_lock_and_nowhere_fancier() {
        let dir = std::env::temp_dir().join(format!(
            "yunari-mcp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(locked_port(&dir), None);
        std::fs::write(dir.join("host.lock"), r#"{"pid":4,"port":5229}"#).unwrap();
        assert_eq!(locked_port(&dir), Some(5229));
        std::fs::write(dir.join("host.lock"), "not json").unwrap();
        assert_eq!(locked_port(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_refusal_is_an_error_object_not_an_empty_line() {
        let line = error_line(0, "connection refused");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["error"]["code"], -32603);
        assert_eq!(value["error"]["message"], "connection refused");
        let line = error_line(503, "");
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value["error"]["message"],
            "the host did not answer (status 503)"
        );
    }
}
