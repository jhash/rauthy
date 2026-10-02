use crate::common::{get_auth_headers, get_backend_url};
use pretty_assertions::assert_eq;
use rauthy_api_types::clients::{ClientResponse, NewClientRequest, UpdateClientRequest};
use std::error::Error;

mod common;

const THEMED_CLIENT: &str = "themed-pages";
const THEMED_CLIENT_URI: &str = "https://themed.client.test/";
const THEMED_RESET_USER: &str = "2PYV3STNz3MN7VnPjJVcPQap";
const THEMED_RESET_LINK: &str = "ThemedResetLinkForClientThemeTests0123456789abcdefghijklmnopqrstu";

async fn create_client_with_uri(id: &str, client_uri: &str) -> Result<(), Box<dyn Error>> {
    let backend = get_backend_url();
    let admin = get_auth_headers().await?;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{backend}/clients"))
        .headers(admin.clone())
        .json(&NewClientRequest {
            id: id.to_string(),
            secret: None,
            name: Some(id.to_string()),
            confidential: false,
            redirect_uris: vec![format!("{client_uri}callback")],
            post_logout_redirect_uris: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let c = res.json::<ClientResponse>().await?;

    let res = client
        .put(format!("{backend}/clients/{id}"))
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
            client_uri: Some(client_uri.to_string()),
            contacts: c.contacts,
            backchannel_logout_uri: c.backchannel_logout_uri,
            restrict_group_prefix: c.restrict_group_prefix,
            claims: None,
            claims_at_root: false,
            allowed_resources: None,
            default_aud: None,
            allowed_providers: None,
            scim: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    Ok(())
}

async fn page(path: &str) -> Result<String, Box<dyn Error>> {
    let res = reqwest::Client::new()
        .get(format!("{}{path}", get_backend_url()))
        .send()
        .await?;
    assert_eq!(res.status(), 200, "{path}");
    Ok(res.text().await?)
}

fn theme_client(html: &str) -> &str {
    let (_, rest) = html
        .split_once("/auth/v1/theme/")
        .expect("page links a theme stylesheet");
    rest.split_once('/').unwrap().0
}

#[tokio::test]
async fn test_register_and_reset_pages_use_the_client_theme() -> Result<(), Box<dyn Error>> {
    create_client_with_uri(THEMED_CLIENT, THEMED_CLIENT_URI).await?;

    let html = page("/users/register").await?;
    assert_eq!(theme_client(&html), "rauthy");

    let html = page("/users/register?redirect_uri=https://unknown.client.test/").await?;
    assert_eq!(theme_client(&html), "rauthy");

    let html = page(&format!("/users/register?redirect_uri={THEMED_CLIENT_URI}")).await?;
    assert_eq!(theme_client(&html), THEMED_CLIENT);

    let authorize = format!(
        "{}/oidc/authorize%3Fclient_id%3D{THEMED_CLIENT}%26redirect_uri%3D{THEMED_CLIENT_URI}callback",
        get_backend_url().replace(':', "%3A").replace('/', "%2F")
    );
    let html = page(&format!("/users/register?redirect_uri={authorize}")).await?;
    assert_eq!(theme_client(&html), THEMED_CLIENT);

    let html = page(&format!(
        "/users/{THEMED_RESET_USER}/reset/{THEMED_RESET_LINK}?type=password_reset"
    ))
    .await?;
    assert_eq!(theme_client(&html), THEMED_CLIENT);

    Ok(())
}
