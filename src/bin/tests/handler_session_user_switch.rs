use crate::common::{
    PASSWORD, USERNAME, cookie_csrf_headers_from_res, get_auth_headers, get_backend_url,
    get_solved_pow, session_headers_with,
};
use pretty_assertions::{assert_eq, assert_ne};
use rauthy_api_types::generic::Language;
use rauthy_api_types::oidc::LoginRequest;
use rauthy_api_types::users::{NewUserRequest, UpdateUserRequest, UserResponse};
use rauthy_common::constants::CSRF_HEADER;
use rauthy_common::sha256;
use rauthy_common::utils::base64_url_encode;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use std::error::Error;

mod common;

const PWD_OTHER: &str = "123SuperSafe123";
const CHALLENGE_PLAIN: &str = "oDXug9zfYqfz8ejcqMpALRPXfW8QhbKV2AVuScAt8xrLKDAmaRYQ4yRi2uqcH9ys";

#[derive(Deserialize)]
struct SessionUser {
    user_id: Option<String>,
}

async fn create_user_with_password(email: &str) -> Result<UserResponse, Box<dyn Error>> {
    let backend = get_backend_url();
    let admin = get_auth_headers().await?;
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{backend}/users"))
        .headers(admin.clone())
        .json(&NewUserRequest {
            given_name: Some("Switch".to_string()),
            family_name: Some("User".to_string()),
            email: email.to_string(),
            preferred_username: None,
            language: Language::En,
            roles: Vec::default(),
            groups: None,
            user_expires: None,
            tz: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let user = res.json::<UserResponse>().await?;

    let res = client
        .put(format!("{backend}/users/{}", user.id))
        .headers(admin)
        .json(&UpdateUserRequest {
            email: user.email.clone(),
            given_name: user.given_name.clone(),
            family_name: user.family_name.clone(),
            language: Some(Language::En),
            password: Some(PWD_OTHER.to_string()),
            roles: user.roles.clone(),
            groups: user.groups.clone(),
            enabled: true,
            email_verified: true,
            user_expires: None,
            user_values: None,
        })
        .send()
        .await?;
    assert_eq!(res.status(), 200);

    Ok(user)
}

fn authorize_query(prompt: Option<&str>) -> String {
    let backend = get_backend_url();
    let challenge = base64_url_encode(sha256!(CHALLENGE_PLAIN.as_bytes()));
    let mut query = format!(
        "{backend}/oidc/authorize?client_id=rauthy&redirect_uri={backend}/oidc/callback\
        &response_type=code&code_challenge={challenge}&code_challenge_method=S256"
    );
    if let Some(prompt) = prompt {
        query.push_str("&prompt=");
        query.push_str(prompt);
    }
    query
}

async fn login_request(email: &str, password: &str) -> LoginRequest {
    let backend = get_backend_url();
    LoginRequest {
        email: email.to_string(),
        password: Some(password.to_string()),
        pow: get_solved_pow().await,
        client_id: "rauthy".to_string(),
        redirect_uri: format!("{backend}/oidc/callback"),
        scopes: None,
        state: None,
        nonce: None,
        code_challenge: Some(base64_url_encode(sha256!(CHALLENGE_PLAIN.as_bytes()))),
        code_challenge_method: Some("S256".to_string()),
        resource: None,
    }
}

async fn session_user_id(headers: &HeaderMap) -> Result<Option<String>, Box<dyn Error>> {
    let res = reqwest::Client::new()
        .get(format!("{}/oidc/sessioninfo", get_backend_url()))
        .headers(headers.clone())
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    Ok(res.json::<SessionUser>().await?.user_id)
}

#[tokio::test]
async fn test_forced_login_starts_new_session_over_another_users_session()
-> Result<(), Box<dyn Error>> {
    let other = create_user_with_password("switch-forced@session.io").await?;
    let admin_session = session_headers_with(USERNAME, PASSWORD).await;
    let admin_id = session_user_id(&admin_session).await?;
    assert!(admin_id.is_some());

    let client = reqwest::Client::new();
    let res = client
        .get(authorize_query(Some("login")))
        .headers(admin_session.clone())
        .send()
        .await?;
    assert_eq!(res.status(), 200);
    let fresh = cookie_csrf_headers_from_res(res).await?;
    assert_ne!(
        fresh.get(CSRF_HEADER),
        admin_session.get(CSRF_HEADER),
        "a forced login must not reuse the session of the user already signed in",
    );

    let res = client
        .post(authorize_query(None))
        .headers(fresh.clone())
        .json(&login_request(&other.email, PWD_OTHER).await)
        .send()
        .await?;
    assert_eq!(res.status(), 202);

    assert_eq!(session_user_id(&fresh).await?, Some(other.id));
    assert_eq!(session_user_id(&admin_session).await?, admin_id);

    Ok(())
}

#[tokio::test]
async fn test_password_login_refuses_session_of_another_user() -> Result<(), Box<dyn Error>> {
    let other = create_user_with_password("switch-password@session.io").await?;
    let admin_session = session_headers_with(USERNAME, PASSWORD).await;
    let admin_id = session_user_id(&admin_session).await?;
    assert!(admin_id.is_some());

    let res = reqwest::Client::new()
        .post(authorize_query(None))
        .headers(admin_session.clone())
        .json(&login_request(&other.email, PWD_OTHER).await)
        .send()
        .await?;
    assert_eq!(res.status(), 403);

    assert_eq!(session_user_id(&admin_session).await?, admin_id);

    Ok(())
}

#[tokio::test]
async fn test_password_login_on_own_session_still_works() -> Result<(), Box<dyn Error>> {
    let admin_session = session_headers_with(USERNAME, PASSWORD).await;
    let admin_id = session_user_id(&admin_session).await?;

    let res = reqwest::Client::new()
        .post(authorize_query(None))
        .headers(admin_session.clone())
        .json(&login_request(USERNAME, PASSWORD).await)
        .send()
        .await?;
    assert_eq!(res.status(), 202);

    assert_eq!(session_user_id(&admin_session).await?, admin_id);

    Ok(())
}
