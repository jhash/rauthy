use crate::common::{
    cookie_csrf_headers_from_res_direct, get_auth_headers, get_backend_url, get_solved_pow,
};
use pretty_assertions::assert_eq;
use rauthy_api_types::clients::{ClientResponse, NewClientRequest, UpdateClientRequest};
use serde_json::{Value, json};
use std::error::Error;

mod common;

const ALLOWLIST_CLIENT: &str = "provider-allowlist";
const PKCE_CHALLENGE: &str = "oDXug9zfYqfz8ejcqMpALRPXfW8QhbKV2AVuScAt8xrLKDAmaRYQ4yRi2uqcH9ys";

async fn create_provider(name: &str) -> Result<String, Box<dyn Error>> {
    let res = reqwest::Client::new()
        .post(format!("{}/providers/create", get_backend_url()))
        .headers(get_auth_headers().await?)
        .json(&json!({
            "name": name,
            "typ": "custom",
            "enabled": true,
            "issuer": format!("https://{name}.upstream.test"),
            "authorization_endpoint": format!("https://{name}.upstream.test/authorize"),
            "token_endpoint": format!("https://{name}.upstream.test/token"),
            "userinfo_endpoint": format!("https://{name}.upstream.test/userinfo"),
            "use_pkce": true,
            "client_secret_basic": false,
            "client_secret_post": true,
            "auto_onboarding": false,
            "auto_link": false,
            "client_id": "rauthy",
            "client_secret": "secret",
            "scope": "openid email",
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    Ok(res.json::<Value>().await?["id"]
        .as_str()
        .unwrap()
        .to_string())
}

async fn create_client_allowing(provider_ids: Vec<String>) -> Result<(), Box<dyn Error>> {
    let backend = get_backend_url();
    let admin = get_auth_headers().await?;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{backend}/clients"))
        .headers(admin.clone())
        .json(&NewClientRequest {
            id: ALLOWLIST_CLIENT.to_string(),
            secret: None,
            name: Some(ALLOWLIST_CLIENT.to_string()),
            confidential: false,
            redirect_uris: vec![format!("{backend}/oidc/callback")],
            post_logout_redirect_uris: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let c = res.json::<ClientResponse>().await?;

    let res = client
        .put(format!("{backend}/clients/{ALLOWLIST_CLIENT}"))
        .headers(admin)
        .json(&UpdateClientRequest {
            name: c.name,
            confidential: c.confidential,
            redirect_uris: c.redirect_uris,
            post_logout_redirect_uris: c.post_logout_redirect_uris,
            allowed_origins: c.allowed_origins,
            enabled: c.enabled,
            flows_enabled: c.flows_enabled,
            access_token_alg: c.access_token_alg,
            id_token_alg: c.id_token_alg,
            auth_code_lifetime: c.auth_code_lifetime,
            access_token_lifetime: c.access_token_lifetime,
            scopes: c.scopes,
            default_scopes: c.default_scopes,
            challenges: c.challenges,
            force_mfa: c.force_mfa,
            client_uri: c.client_uri,
            contacts: c.contacts,
            backchannel_logout_uri: c.backchannel_logout_uri,
            restrict_group_prefix: c.restrict_group_prefix,
            claims: None,
            claims_at_root: false,
            allowed_resources: None,
            default_aud: None,
            allowed_providers: Some(provider_ids.clone()),
            scim: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let updated = res.json::<ClientResponse>().await?;
    assert_eq!(updated.allowed_providers, Some(provider_ids));
    Ok(())
}

async fn login_page_providers(client_id: &str) -> Result<String, Box<dyn Error>> {
    let backend = get_backend_url();
    let res = reqwest::Client::new()
        .get(format!(
            "{backend}/oidc/authorize?client_id={client_id}&redirect_uri={backend}/oidc/callback\
            &response_type=code&code_challenge={PKCE_CHALLENGE}&code_challenge_method=S256"
        ))
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let html = res.text().await?;
    let (_, rest) = html
        .split_once(r#"<template id="tpl_auth_providers">"#)
        .unwrap();
    Ok(rest.split_once("</template>").unwrap().0.to_string())
}

async fn provider_login(provider_id: &str) -> Result<u16, Box<dyn Error>> {
    let backend = get_backend_url();
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{backend}/oidc/session"))
        .send()
        .await?;
    let headers = cookie_csrf_headers_from_res_direct(res).await?;

    let res = client
        .post(format!("{backend}/providers/login"))
        .headers(headers)
        .json(&json!({
            "client_id": ALLOWLIST_CLIENT,
            "redirect_uri": format!("{backend}/oidc/callback"),
            "code_challenge": PKCE_CHALLENGE,
            "code_challenge_method": "S256",
            "pow": get_solved_pow().await,
            "provider_id": provider_id,
            "pkce_challenge": PKCE_CHALLENGE,
        }))
        .send()
        .await?;
    Ok(res.status().as_u16())
}

#[tokio::test]
async fn test_client_shows_and_accepts_only_allowed_providers() -> Result<(), Box<dyn Error>> {
    let allowed = create_provider("allowed").await?;
    let other = create_provider("other").await?;
    create_client_allowing(vec![allowed.clone()]).await?;

    let everyone = login_page_providers("rauthy").await?;
    assert!(everyone.contains(&allowed), "{everyone}");
    assert!(everyone.contains(&other), "{everyone}");

    let restricted = login_page_providers(ALLOWLIST_CLIENT).await?;
    assert!(restricted.contains(&allowed), "{restricted}");
    assert!(!restricted.contains(&other), "{restricted}");

    assert_eq!(provider_login(&other).await?, 403);
    assert_eq!(provider_login(&allowed).await?, 202);

    Ok(())
}
