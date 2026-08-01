use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::setup::prompt;

const TIMER_UNIT: &str = "/etc/systemd/system/irixmail-update.timer";

pub fn configure() -> Result<()> {
    if !Path::new(TIMER_UNIT).exists() {
        return Ok(());
    }
    let answer = prompt("Enable automatic daily updates? [Y/n]: ")?;
    if !wants_auto_updates(&answer) {
        println!("Automatic updates stay off; update manually with `sudo irixmail update`.");
        return Ok(());
    }
    match Command::new("systemctl")
        .args(["enable", "--now", "irixmail-update.timer"])
        .status()
    {
        Ok(status) if status.success() => println!("Automatic updates enabled."),
        _ => println!(
            "Could not enable the timer; run: sudo systemctl enable --now irixmail-update.timer"
        ),
    }
    Ok(())
}

fn wants_auto_updates(answer: &str) -> bool {
    matches!(answer.to_ascii_lowercase().as_str(), "" | "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::wants_auto_updates;

    #[test]
    fn automatic_updates_default_to_yes() {
        assert!(wants_auto_updates(""));
        assert!(wants_auto_updates("y"));
        assert!(wants_auto_updates("Yes"));
        assert!(!wants_auto_updates("n"));
        assert!(!wants_auto_updates("no"));
        assert!(!wants_auto_updates("whatever"));
    }
}
