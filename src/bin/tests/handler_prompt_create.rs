use crate::common::{get_backend_url, get_solved_pow};
use pretty_assertions::assert_eq;
use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use std::error::Error;

mod common;

const PKCE_CHALLENGE: &str = "oDXug9zfYqfz8ejcqMpALRPXfW8QhbKV2AVuScAt8xrLKDAmaRYQ4yRi2uqcH9ys";

fn authorize_url(extra: &str) -> String {
    let backend = get_backend_url();
    format!(
        "{backend}/oidc/authorize?client_id=rauthy&redirect_uri={backend}/oidc/callback\
        &response_type=code&code_challenge={PKCE_CHALLENGE}&code_challenge_method=S256{extra}"
    )
}

fn decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn test_prompt_create_opens_registration_and_returns_to_authorize()
-> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()?;
    let res = client
        .get(authorize_url("&state=abc&prompt=create"))
        .send()
        .await?;
    assert_eq!(res.status(), 302);
    let location = res.headers().get(LOCATION).unwrap().to_str()?.to_string();

    let (path, query) = location.split_once('?').unwrap();
    assert_eq!(path, "/auth/v1/users/register");
    let (key, value) = query.split_once('=').unwrap();
    assert_eq!(key, "redirect_uri");
    assert_eq!(decode(value), authorize_url("&state=abc"));
    Ok(())
}

#[tokio::test]
async fn test_registration_accepts_own_authorize_url_as_redirect() -> Result<(), Box<dyn Error>> {
    let backend = get_backend_url();
    let res = reqwest::Client::new()
        .post(format!("{backend}/users/register"))
        .json(&json!({
            "email": "prompt-create@register.test",
            "given_name": "Prompt",
            "family_name": "Create",
            "pow": get_solved_pow().await,
            "redirect_uri": authorize_url("&state=abc"),
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 204, "{}", res.text().await?);

    let res = reqwest::Client::new()
        .post(format!("{backend}/users/register"))
        .json(&json!({
            "email": "prompt-create-other@register.test",
            "given_name": "Prompt",
            "family_name": "Create",
            "pow": get_solved_pow().await,
            "redirect_uri": "https://evil.test/auth/v1/oidc/authorize?client_id=rauthy",
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 400);
    Ok(())
}

#[tokio::test]
async fn test_discovery_advertises_prompt_create() -> Result<(), Box<dyn Error>> {
    let res = reqwest::get(format!(
        "{}/.well-known/openid-configuration",
        get_backend_url()
    ))
    .await?;
    assert_eq!(res.status(), 200);
    let doc = res.json::<Value>().await?;
    let prompts = doc["prompt_values_supported"].as_array().unwrap();
    for prompt in ["none", "login", "consent", "create"] {
        assert!(prompts.iter().any(|p| p == prompt), "{prompts:?}");
    }
    Ok(())
}
