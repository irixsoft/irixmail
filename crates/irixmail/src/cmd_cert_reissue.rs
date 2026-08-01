use std::process::Command;

use anyhow::{Context, Result};
use irixmail_core::BootstrapConfig;

use crate::setup::prompt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReissuePlan {
    Direct,
    StopIssueRestart,
    NeedsRoot,
    PortBusyElsewhere,
}

fn plan(bind_failed: bool, service_active: bool, is_root: bool) -> ReissuePlan {
    match (bind_failed, service_active, is_root) {
        (false, _, _) => ReissuePlan::Direct,
        (true, true, true) => ReissuePlan::StopIssueRestart,
        (true, true, false) => ReissuePlan::NeedsRoot,
        (true, false, _) => ReissuePlan::PortBusyElsewhere,
    }
}

fn stop_confirmed(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
}

fn service_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", "irixmail"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

struct RestartOnDrop;

impl Drop for RestartOnDrop {
    fn drop(&mut self) {
        if systemctl(&["start", "irixmail"]) {
            println!("Service irixmail restarted.");
        } else {
            println!("Could not restart the service; run: sudo systemctl start irixmail");
        }
    }
}

fn issue(config: &BootstrapConfig) -> Result<()> {
    println!("Requesting a certificate for {}...", config.server.hostname);
    if crate::setup_cert::obtain(config, None)? {
        println!("Certificate reissued.");
    } else {
        println!("Reissue did not complete — check DNS and that port 80 is reachable.");
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let config_file = crate::cmd_run::config_path();
    let config = BootstrapConfig::load(&config_file)
        .with_context(|| format!("loading configuration from {}", config_file.display()))?;
    let port = config.listeners.http.plain.unwrap_or(80);

    match plan(
        !irixmail_tls::port_is_bindable(port),
        service_active(),
        crate::ownership::running_as_root(),
    ) {
        ReissuePlan::NeedsRoot => {
            println!(
                "irixmail is running and holds port {port}; re-run as root: sudo irixmail cert reissue"
            );
            Ok(())
        }
        ReissuePlan::PortBusyElsewhere => {
            println!("port {port} is in use by another process; stop it and retry");
            Ok(())
        }
        ReissuePlan::Direct => {
            issue(&config)?;
            crate::ownership::ensure_service_ownership(&config, &config_file)
        }
        ReissuePlan::StopIssueRestart => {
            println!(
                "irixmail is running and holds port {port} — mail service pauses for about a minute."
            );
            let answer = prompt("Stop it briefly to issue the certificate? [Y/n]: ")
                .unwrap_or_else(|_| "n".to_string());
            if !stop_confirmed(&answer) {
                println!("Left the service running. Reissue from the admin panel instead.");
                return Ok(());
            }
            if !systemctl(&["stop", "irixmail"]) {
                anyhow::bail!("could not stop the irixmail service");
            }
            let _restart = RestartOnDrop;
            issue(&config)?;
            crate::ownership::ensure_service_ownership(&config, &config_file)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_challenge_port_issues_in_place() {
        assert_eq!(plan(false, true, true), ReissuePlan::Direct);
        assert_eq!(plan(false, false, false), ReissuePlan::Direct);
    }

    #[test]
    fn a_busy_port_held_by_the_running_service_stops_and_restarts_it() {
        assert_eq!(plan(true, true, true), ReissuePlan::StopIssueRestart);
    }

    #[test]
    fn a_busy_port_without_root_asks_for_sudo() {
        assert_eq!(plan(true, true, false), ReissuePlan::NeedsRoot);
    }

    #[test]
    fn a_busy_port_without_the_service_reports_the_conflict() {
        assert_eq!(plan(true, false, true), ReissuePlan::PortBusyElsewhere);
    }

    #[test]
    fn an_empty_answer_confirms_the_stop() {
        assert!(stop_confirmed(""));
        assert!(stop_confirmed("Y\n"));
        assert!(stop_confirmed("yes"));
        assert!(!stop_confirmed("n"));
        assert!(!stop_confirmed("no"));
    }
}
