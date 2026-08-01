use std::io::{self, BufRead, IsTerminal, Write};

use anyhow::{Context, Result};
use irixmail_core::BootstrapConfig;

pub fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().context("flushing the prompt")?;
    read_input(&mut io::stdin().lock())
}

fn read_input(reader: &mut impl BufRead) -> Result<String> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).context("reading input")?;
    if bytes == 0 {
        anyhow::bail!("input ended unexpectedly; rerun the setup in an interactive terminal");
    }
    Ok(line.trim().to_string())
}

pub fn run() -> Result<()> {
    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "the setup needs an interactive terminal; run it directly: sudo irixmail setup"
        );
    }
    println!("IRIXMAIL interactive setup");
    let path = crate::cmd_run::config_path();
    let mut config = match BootstrapConfig::load(&path) {
        Ok(existing) => {
            println!(
                "Existing configuration found at {} — its settings are kept.",
                path.display()
            );
            existing
        }
        Err(irixmail_core::Error::NotFound(_)) => BootstrapConfig::default(),
        Err(error) => {
            return Err(anyhow::anyhow!("{error}"))
                .with_context(|| format!("loading configuration from {}", path.display()))
        }
    };
    crate::setup_hostname::configure(&mut config)?;
    crate::setup_relay::configure(&mut config)?;

    config
        .save(&path)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .with_context(|| format!("writing configuration to {}", path.display()))?;
    println!("Configuration written to {}", path.display());

    let admin_email = crate::setup_admin::configure(&config)?;
    let addresses = crate::setup_ip::display()?;
    crate::setup_dns::configure(&config, &addresses)?;
    let cert_issued = crate::setup_cert::configure(&config, &admin_email)?;
    crate::setup_updates::configure()?;
    crate::setup_finish::configure(&config, cert_issued)?;
    crate::ownership::ensure_service_ownership(&config, &path)?;
    crate::setup_service::configure()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::read_input;
    use std::io::Cursor;

    #[test]
    fn a_closed_input_stream_is_an_error_not_an_empty_answer() {
        let error = read_input(&mut Cursor::new("")).unwrap_err();
        assert!(error.to_string().contains("interactive terminal"));
    }

    #[test]
    fn a_line_of_input_is_returned_trimmed() {
        let answer = read_input(&mut Cursor::new("  mail.example.com\n")).unwrap();
        assert_eq!(answer, "mail.example.com");
    }
}
