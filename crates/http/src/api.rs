use axum::response::Redirect;
use axum::routing::{any, get, post};
use axum::Router;

use crate::app::AppState;

pub fn routes(state: AppState) -> Router<AppState> {
    public_routes()
        .merge(me_routes(state.clone()))
        .merge(admin_routes(state))
}

fn me_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/api/me/password",
            axum::routing::put(crate::me::change_password),
        )
        .route(
            "/api/me/app-passwords",
            get(crate::me::list_app_passwords).post(crate::me::create_app_password),
        )
        .route(
            "/api/me/app-passwords/{pid}",
            axum::routing::delete(crate::me::delete_app_password),
        )
        .route("/api/me/totp", get(crate::me_totp::status))
        .route("/api/me/totp/setup", post(crate::me_totp::setup))
        .route("/api/me/totp/verify", post(crate::me_totp::verify))
        .route("/api/me/totp/disable", post(crate::me_totp::disable))
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth_mw::require_interactive,
        ))
}

pub(crate) fn public_routes() -> Router<AppState> {
    let mut router = Router::new()
        .route("/api/auth/login", post(crate::auth_login::login))
        .route("/api/auth/totp", post(crate::auth_totp::totp))
        .route("/api/auth/logout", post(crate::auth_logout::logout))
        .route("/healthz/live", get(crate::health_live::live))
        .route("/healthz/ready", get(crate::health_ready::ready))
        .route("/.well-known/jmap", get(crate::wk_jmap::well_known_jmap))
        .route(
            "/.well-known/acme-challenge/{token}",
            get(crate::wk_acme::acme_challenge),
        )
        .route("/.well-known/mta-sts.txt", get(crate::wk_mtasts::mta_sts))
        .route(
            "/.well-known/caldav",
            any(|| async { Redirect::temporary("/dav/cal/") }),
        )
        .route(
            "/.well-known/carddav",
            any(|| async { Redirect::temporary("/dav/card/") }),
        )
        .route("/mail/config-v1.1.xml", get(crate::autoconfig::autoconfig))
        .route(
            "/.well-known/autoconfig/mail/config-v1.1.xml",
            get(crate::autoconfig::autoconfig_well_known),
        )
        .route(
            "/.well-known/mail-v1.xml",
            get(crate::autoconfig::autoconfig),
        );
    for prefix in ["autodiscover", "Autodiscover", "AutoDiscover"] {
        for document in ["autodiscover", "Autodiscover", "AutoDiscover"] {
            router = router
                .route(
                    &format!("/{prefix}/{document}.xml"),
                    post(crate::autodiscover::autodiscover),
                )
                .route(
                    &format!("/{prefix}/{document}.json"),
                    any(crate::autodiscover::autodiscover_json),
                )
                .route(
                    &format!("/{prefix}/{document}.json/{{*rest}}"),
                    any(crate::autodiscover::autodiscover_json),
                );
        }
    }
    router
}

fn admin_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/dashboard", get(crate::dashboard::dashboard))
        .route(
            "/api/domains",
            get(crate::domains_list::list).post(crate::domains_create::create),
        )
        .route(
            "/api/domains/{id}",
            get(crate::domains_get::get)
                .put(crate::domains_update::update)
                .delete(crate::domains_delete::delete),
        )
        .route("/api/domains/{id}/dns", get(crate::domains_dns::dns))
        .route(
            "/api/domains/{id}/dns/verify",
            post(crate::domains_dns_verify::verify),
        )
        .route("/api/domains/{id}/dkim", get(crate::domains_dkim::dkim))
        .route(
            "/api/accounts",
            get(crate::accounts_list::list).post(crate::accounts_create::create),
        )
        .route(
            "/api/accounts/{id}",
            get(crate::accounts_get::get)
                .put(crate::accounts_update::update)
                .delete(crate::accounts_delete::delete),
        )
        .route(
            "/api/accounts/{id}/password",
            axum::routing::put(crate::accounts_password::set),
        )
        .route(
            "/api/accounts/{id}/aliases",
            get(crate::aliases_list::list).post(crate::aliases_create::create),
        )
        .route(
            "/api/accounts/{id}/aliases/{alias}",
            axum::routing::delete(crate::aliases_delete::delete),
        )
        .route(
            "/api/accounts/{id}/forwarding",
            get(crate::forwarding_get::get).put(crate::forwarding_set::set),
        )
        .route(
            "/api/accounts/{id}/app-passwords",
            get(crate::apppw_list::list).post(crate::apppw_create::create),
        )
        .route(
            "/api/accounts/{id}/app-passwords/{pid}",
            axum::routing::delete(crate::apppw_delete::delete),
        )
        .route(
            "/api/accounts/{id}/reset-2fa",
            post(crate::reset_2fa::reset),
        )
        .route(
            "/api/accounts/{id}/reindex",
            post(crate::accounts_reindex::reindex),
        )
        .route("/api/queue", get(crate::queue_list::list))
        .route("/api/queue/{id}/retry", post(crate::queue_retry::retry))
        .route(
            "/api/queue/{id}",
            axum::routing::delete(crate::queue_delete::delete),
        )
        .route("/api/logs", get(crate::logs::logs))
        .route(
            "/api/ip-rules",
            get(crate::ip_rules_list::list).post(crate::ip_rules_create::create),
        )
        .route(
            "/api/ip-rules/{id}",
            axum::routing::delete(crate::ip_rules_delete::delete),
        )
        .route("/api/tls", get(crate::tls_get::get))
        .route("/api/tls/upload", post(crate::tls_upload::upload))
        .route("/api/tls/reissue", post(crate::tls_reissue::reissue))
        .route(
            "/api/settings",
            get(crate::settings_get::get).put(crate::settings_put::put),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth_mw::require_admin,
        ))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::app::router;
    use crate::tests_support::{state, TempDir};

    #[tokio::test]
    async fn the_well_known_routes_are_mounted() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/.well-known/mta-sts.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_autoconfig_route_is_mounted() {
        let dir = TempDir::new();
        let app = router(state(&dir));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/mail/config-v1.1.xml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
