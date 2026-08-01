use std::net::IpAddr;

use anyhow::{Context, Result};
use irixmail_dns::public_ip;

pub fn display() -> Result<Vec<IpAddr>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building the tokio runtime")?;
    let addresses = runtime.block_on(public_ip::detect_all());
    if addresses.is_empty() {
        println!("Could not detect a public IP address automatically.");
    } else {
        println!("Detected server IP address(es):");
        for address in &addresses {
            println!("  {address}");
        }
    }
    Ok(addresses)
}
