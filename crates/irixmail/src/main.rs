mod cli;
mod cmd_api_key;
mod cmd_backup;
mod cmd_cert_reissue;
mod cmd_cert_status;
mod cmd_reset_password;
mod cmd_restore;
mod cmd_run;
mod cmd_update;
mod ownership;
mod setup;
mod setup_admin;
mod setup_cert;
mod setup_dns;
mod setup_finish;
mod setup_hostname;
mod setup_ip;
mod setup_relay;
mod setup_service;
mod setup_updates;

fn main() {
    // feature unification compiles rustls with ring AND aws-lc-rs; unpinned, TLS clients panic
    let _ = irixmail_tls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    if let Err(error) = cli::run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
