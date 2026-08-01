use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::setup::prompt;

const SERVICE_UNIT: &str = "/etc/systemd/system/irixmail.service";

pub fn configure() -> Result<()> {
    if !Path::new(SERVICE_UNIT).exists() {
        return Ok(());
    }
    let answer = prompt("Start irixmail now and on boot? [Y/n]: ")?;
    if !wants_service_start(&answer) {
        println!("Start it later with: sudo systemctl enable --now irixmail");
        return Ok(());
    }
    match Command::new("systemctl")
        .args(["enable", "--now", "irixmail"])
        .status()
    {
        Ok(status) if status.success() => println!("irixmail is running and enabled on boot."),
        _ => println!("Could not start the service; run: sudo systemctl enable --now irixmail"),
    }
    Ok(())
}

fn wants_service_start(answer: &str) -> bool {
    matches!(answer.to_ascii_lowercase().as_str(), "" | "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::wants_service_start;

    #[test]
    fn starting_the_service_defaults_to_yes() {
        assert!(wants_service_start(""));
        assert!(wants_service_start("y"));
        assert!(wants_service_start("Yes"));
        assert!(!wants_service_start("n"));
        assert!(!wants_service_start("no"));
        assert!(!wants_service_start("whatever"));
    }
}
