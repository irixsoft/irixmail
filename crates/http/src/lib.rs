pub mod accounts_create;
pub mod accounts_delete;
pub mod accounts_get;
pub mod accounts_list;
pub mod accounts_password;
pub mod accounts_reindex;
pub mod accounts_update;
pub mod aliases_create;
pub mod aliases_delete;
pub mod aliases_list;
pub mod api;
pub mod app;
pub mod apppw_create;
pub mod apppw_delete;
pub mod apppw_list;
pub mod auth_login;
pub mod auth_logout;
pub mod auth_mw;
pub mod auth_totp;
pub mod autoconfig;
pub mod autodiscover;
pub mod dashboard;
pub mod dav_mount;
pub mod dns_status;
pub mod domains_create;
pub mod domains_delete;
pub mod domains_dkim;
pub mod domains_dns;
pub mod domains_dns_verify;
pub mod domains_get;
pub mod domains_list;
pub mod domains_update;
pub mod forwarding_get;
pub mod forwarding_set;
pub mod health_live;
pub mod health_ready;
pub mod ip_rules_create;
pub mod ip_rules_delete;
pub mod ip_rules_list;
pub mod jmap_mount;
pub mod logs;
pub mod me;
pub mod me_totp;
pub mod push_worker;
pub mod queue_delete;
pub mod queue_list;
pub mod queue_retry;
pub mod reset_2fa;
pub mod serve;
pub mod settings_get;
pub mod settings_put;
#[path = "static.rs"]
pub mod static_assets;
pub mod tls_get;
pub mod tls_reissue;
pub mod tls_upload;
pub mod validate;
pub mod wk_acme;
pub mod wk_jmap;
pub mod wk_mtasts;

#[cfg(test)]
mod tests_support;

pub use accounts_list::account_json;
pub use app::{error_response, router, AppState, SessionTokens, TlsHandles, TokenInfo};
pub use auth_mw::{authenticate_request, require_admin, require_auth, AuthIdentity};
pub use dns_status::{recheck_all, RecheckInput};
pub use domains_list::domain_json;
pub use jmap_mount::methods as jmap_methods;
pub use serve::{redirect_router, register_http, register_http_redirect, register_https};
pub use static_assets::{rewrite_base_href, serve_asset, spa_fallback};
pub use validate::{
    bad_request, is_valid_domain, is_valid_email, parse_id, require_field, unprocessable,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
