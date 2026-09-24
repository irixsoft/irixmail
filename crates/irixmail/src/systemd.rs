use std::process::Command;

pub fn service_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", "irixmail"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl")
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub struct RestartOnDrop;

impl Drop for RestartOnDrop {
    fn drop(&mut self) {
        if systemctl(&["start", "irixmail"]) {
            println!("Service irixmail restarted.");
        } else {
            println!("Could not restart the service; run: sudo systemctl start irixmail");
        }
    }
}
