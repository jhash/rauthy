use crate::common::{get_auth_headers, get_backend_url, get_solved_pow};
use cryptr::utils::secure_random_alnum;
use pretty_assertions::{assert_eq, assert_ne};
use rauthy_api_types::generic::Language;
use rauthy_api_types::oidc::LoginRequest;
use rauthy_api_types::users::{NewUserRequest, UpdateUserRequest, UserResponse};
use rauthy_common::constants::CSRF_HEADER;
use rauthy_common::sha256;
use rauthy_common::utils::base64_url_encode;
use reqwest::header::{self, HeaderMap, HeaderValue};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod common;

type TestResult = Result<(), Box<dyn Error>>;

const PKCE_VERIFIER: &str = "oDXug9zfYqfz8ejcqMpALRPXfW8QhbKV2AVuScAt8xrLKDAmaRYQ4yRi2uqcH9ys";
const PASSWORD: &str = "123SuperSafe123";

fn unique(prefix: &str) -> String {
    format!("{prefix}{}", secure_random_alnum(10).to_lowercase())
}

fn identity(subject: &str, email: &str) -> String {
    format!("{subject}~{email}")
}

async fn start_mock_upstream() -> Result<String, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_mock(stream));
        }
    });
    Ok(format!("http://{addr}"))
}

async fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
        let len = header_value(&head, "content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < pos + 4 + len {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let body = String::from_utf8_lossy(&buf[pos + 4..pos + 4 + len]).to_string();
        return Some((head, body));
    }
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

async fn serve_mock(mut stream: TcpStream) {
    let Some((head, body)) = read_request(&mut stream).await else {
        return;
    };
    let request_line = head.lines().next().unwrap_or_default();
    let (status, json) = if request_line.starts_with("POST /token") {
        let code = body
            .split('&')
            .find_map(|kv| kv.strip_prefix("code="))
            .map(percent_decode)
            .unwrap_or_default();
        (200, json!({ "access_token": code, "token_type": "Bearer" }))
    } else if request_line.starts_with("GET /userinfo") {
        let token = header_value(&head, "authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or_default();
        let (subject, email) = token.split_once('~').unwrap_or_default();
        (
            200,
            json!({
                "sub": subject,
                "email": email,
                "email_verified": true,
                "given_name": "Linked",
            }),
        )
    } else {
        (404, json!({ "error": "not_found" }))
    };

    let body = json.to_string();
    let res = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
        Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(res.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn create_provider(mock: &str, auto_onboarding: bool) -> Result<String, Box<dyn Error>> {
    let name = unique("Upstream ");
    let res = Client::new()
        .post(format!("{}/providers/create", get_backend_url()))
        .headers(get_auth_headers().await?)
        .json(&json!({
            "name": name,
            "typ": "custom",
            "enabled": true,
            "issuer": format!("{mock}/{}", name.replace(' ', "")),
            "authorization_endpoint": format!("{mock}/authorize"),
            "token_endpoint": format!("{mock}/token"),
            "userinfo_endpoint": format!("{mock}/userinfo"),
            "use_pkce": true,
            "client_secret_basic": false,
            "client_secret_post": false,
            "auto_onboarding": auto_onboarding,
            "auto_link": false,
            "client_id": "mock-client",
            "scope": "openid email",
        }))
        .send()
        .await?;
    let res = expect_status(res, 200).await?;
    Ok(res.json::<Value>().await?["id"]
        .as_str()
        .ok_or("provider id")?
        .to_string())
}

async fn create_password_user(email: &str) -> Result<String, Box<dyn Error>> {
    let backend = get_backend_url();
    let admin = get_auth_headers().await?;
    let client = Client::new();
    let res = client
        .post(format!("{backend}/users"))
        .headers(admin.clone())
        .json(&NewUserRequest {
            given_name: Some("Linked".to_string()),
            family_name: None,
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
    let user = expect_status(res, 200)
        .await?
        .json::<UserResponse>()
        .await?;
    let res = client
        .put(format!("{backend}/users/{}", user.id))
        .headers(admin)
        .json(&UpdateUserRequest {
            email: user.email.clone(),
            given_name: user.given_name.clone(),
            family_name: user.family_name.clone(),
            language: Some(Language::En),
            password: Some(PASSWORD.to_string()),
            roles: user.roles.clone(),
            groups: user.groups.clone(),
            enabled: true,
            email_verified: true,
            user_expires: None,
            user_values: None,
        })
        .send()
        .await?;
    expect_status(res, 200).await?;
    Ok(user.id)
}

async fn expect_status(res: Response, status: u16) -> Result<Response, Box<dyn Error>> {
    if res.status().as_u16() != status {
        let got = res.status();
        let text = res.text().await.unwrap_or_default();
        return Err(format!("expected HTTP {status}, got {got}: {text}").into());
    }
    Ok(res)
}

struct Browser {
    client: Client,
    cookies: BTreeMap<String, String>,
    csrf: String,
}

struct Started {
    state: String,
    xsrf_token: String,
}

impl Browser {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let mut slf = Self {
            client: Client::new(),
            cookies: BTreeMap::new(),
            csrf: String::new(),
        };
        let res = slf
            .client
            .post(format!("{}/oidc/session", get_backend_url()))
            .send()
            .await?;
        slf.take_cookies(&res);
        let info = expect_status(res, 201).await?.json::<Value>().await?;
        slf.csrf = info["csrf_token"].as_str().ok_or("csrf token")?.to_string();
        Ok(slf)
    }

    fn take_cookies(&mut self, res: &Response) {
        for value in res.headers().get_all(header::SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let pair = value.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if value.is_empty() {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_string(), value.to_string());
            }
        }
    }

    fn headers(&self) -> Result<HeaderMap, Box<dyn Error>> {
        let cookie = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(&cookie)?);
        headers.insert(CSRF_HEADER, HeaderValue::from_str(&self.csrf)?);
        Ok(headers)
    }

    async fn password_login(&mut self, email: &str) -> TestResult {
        let backend = get_backend_url();
        let challenge = base64_url_encode(sha256!(PKCE_VERIFIER.as_bytes()));
        let res = self
            .client
            .post(format!("{backend}/oidc/authorize"))
            .headers(self.headers()?)
            .json(&LoginRequest {
                email: email.to_string(),
                password: Some(PASSWORD.to_string()),
                pow: get_solved_pow().await,
                client_id: "rauthy".to_string(),
                redirect_uri: format!("{backend}/oidc/callback"),
                scopes: None,
                state: None,
                nonce: None,
                code_challenge: Some(challenge),
                code_challenge_method: Some("S256".to_string()),
                resource: None,
            })
            .send()
            .await?;
        self.take_cookies(&res);
        expect_status(res, 202).await?;
        Ok(())
    }

    async fn start(&mut self, path: &str, provider_id: &str) -> Result<Response, Box<dyn Error>> {
        let backend = get_backend_url();
        let challenge = base64_url_encode(sha256!(PKCE_VERIFIER.as_bytes()));
        let res = self
            .client
            .post(format!("{backend}{path}"))
            .headers(self.headers()?)
            .json(&json!({
                "client_id": "rauthy",
                "redirect_uri": format!("{backend}/oidc/callback"),
                "code_challenge": challenge,
                "code_challenge_method": "S256",
                "pow": get_solved_pow().await,
                "provider_id": provider_id,
                "pkce_challenge": challenge,
            }))
            .send()
            .await?;
        self.take_cookies(&res);
        Ok(res)
    }

    async fn started(res: Response) -> Result<Started, Box<dyn Error>> {
        let res = expect_status(res, 202).await?;
        let location = res
            .headers()
            .get(header::LOCATION)
            .ok_or("location")?
            .to_str()?
            .to_string();
        let state = location
            .split_once('?')
            .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("state=")))
            .ok_or("state")?
            .to_string();
        Ok(Started {
            state,
            xsrf_token: res.text().await?,
        })
    }

    async fn callback(
        &mut self,
        started: &Started,
        code: &str,
    ) -> Result<Response, Box<dyn Error>> {
        let res = self
            .client
            .post(format!("{}/providers/callback", get_backend_url()))
            .headers(self.headers()?)
            .json(&json!({
                "state": started.state,
                "code": code,
                "xsrf_token": started.xsrf_token,
                "pkce_verifier": PKCE_VERIFIER,
            }))
            .send()
            .await?;
        self.take_cookies(&res);
        Ok(res)
    }

    async fn sign_in(&mut self, provider_id: &str, code: &str) -> Result<Response, Box<dyn Error>> {
        let res = self.start("/providers/login", provider_id).await?;
        let started = Self::started(res).await?;
        self.callback(&started, code).await
    }

    async fn link(&mut self, provider_id: &str, code: &str) -> Result<Response, Box<dyn Error>> {
        let res = self
            .start(&format!("/providers/{provider_id}/link"), provider_id)
            .await?;
        if res.status().as_u16() != 202 {
            return Ok(res);
        }
        let started = Self::started(res).await?;
        self.callback(&started, code).await
    }

    async fn user_id(&self) -> Result<String, Box<dyn Error>> {
        let res = self
            .client
            .get(format!("{}/oidc/sessioninfo", get_backend_url()))
            .headers(self.headers()?)
            .send()
            .await?;
        let info = expect_status(res, 200).await?.json::<Value>().await?;
        Ok(info["user_id"].as_str().ok_or("user id")?.to_string())
    }

    async fn links(&self) -> Result<Vec<Value>, Box<dyn Error>> {
        let res = self
            .client
            .get(format!("{}/providers/links", get_backend_url()))
            .headers(self.headers()?)
            .send()
            .await?;
        Ok(expect_status(res, 200).await?.json::<Vec<Value>>().await?)
    }

    async fn unlink(&self, provider_id: &str) -> Result<Response, Box<dyn Error>> {
        Ok(self
            .client
            .delete(format!(
                "{}/providers/{provider_id}/link",
                get_backend_url()
            ))
            .headers(self.headers()?)
            .send()
            .await?)
    }
}

async fn signed_in_user(provider_id: &str, code: &str) -> Result<String, Box<dyn Error>> {
    let mut browser = Browser::new().await?;
    let res = browser.sign_in(provider_id, code).await?;
    if res.status().as_u16() != 205 {
        expect_status(res, 202).await?;
    }
    browser.user_id().await
}

#[tokio::test]
async fn test_two_providers_sign_in_as_the_same_user() -> TestResult {
    let mock = start_mock_upstream().await?;
    let google = create_provider(&mock, false).await?;
    let github = create_provider(&mock, false).await?;
    let email = format!("{}@links.test", unique("two"));
    let user_id = create_password_user(&email).await?;

    let mut browser = Browser::new().await?;
    browser.password_login(&email).await?;
    expect_status(browser.link(&google, &identity("g-1", &email)).await?, 204).await?;
    expect_status(browser.link(&github, &identity("gh-1", &email)).await?, 204).await?;

    let mut linked = browser
        .links()
        .await?
        .into_iter()
        .map(|l| l["provider_id"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    linked.sort();
    let mut expected = vec![google.clone(), github.clone()];
    expected.sort();
    assert_eq!(linked, expected);

    assert_eq!(
        signed_in_user(&google, &identity("g-1", &email)).await?,
        user_id
    );
    assert_eq!(
        signed_in_user(&github, &identity("gh-1", &email)).await?,
        user_id
    );
    Ok(())
}

#[tokio::test]
async fn test_same_provider_cannot_be_linked_twice() -> TestResult {
    let mock = start_mock_upstream().await?;
    let google = create_provider(&mock, false).await?;
    let email = format!("{}@links.test", unique("twice"));
    create_password_user(&email).await?;

    let mut browser = Browser::new().await?;
    browser.password_login(&email).await?;
    expect_status(browser.link(&google, &identity("g-2", &email)).await?, 204).await?;
    let res = browser.link(&google, &identity("g-3", &email)).await?;
    assert_eq!(res.status().as_u16(), 400);
    assert_eq!(browser.links().await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn test_identity_of_another_user_cannot_be_linked() -> TestResult {
    let mock = start_mock_upstream().await?;
    let google = create_provider(&mock, false).await?;
    let first = format!("{}@links.test", unique("first"));
    let second = format!("{}@links.test", unique("second"));
    let first_id = create_password_user(&first).await?;
    create_password_user(&second).await?;

    let mut browser = Browser::new().await?;
    browser.password_login(&first).await?;
    expect_status(browser.link(&google, &identity("g-4", &first)).await?, 204).await?;

    let mut other = Browser::new().await?;
    other.password_login(&second).await?;
    let res = other.link(&google, &identity("g-4", &second)).await?;
    assert_ne!(res.status().as_u16(), 204);
    assert!(other.links().await?.is_empty());

    assert_eq!(
        signed_in_user(&google, &identity("g-4", &first)).await?,
        first_id
    );
    Ok(())
}

#[tokio::test]
async fn test_unlink_one_provider_keeps_the_last_way_in() -> TestResult {
    let mock = start_mock_upstream().await?;
    let onboarding = create_provider(&mock, true).await?;
    let second = create_provider(&mock, false).await?;
    let email = format!("{}@links.test", unique("unlink"));

    let mut browser = Browser::new().await?;
    let res = browser
        .sign_in(&onboarding, &identity("o-1", &email))
        .await?;
    if res.status().as_u16() != 205 {
        expect_status(res, 202).await?;
    }
    let user_id = browser.user_id().await?;
    expect_status(browser.link(&second, &identity("s-1", &email)).await?, 204).await?;
    assert_eq!(browser.links().await?.len(), 2);

    expect_status(browser.unlink(&onboarding).await?, 200).await?;
    assert_eq!(browser.links().await?.len(), 1);
    assert_eq!(browser.unlink(&second).await?.status().as_u16(), 400);
    assert_eq!(browser.links().await?.len(), 1);

    assert_eq!(
        signed_in_user(&second, &identity("s-1", &email)).await?,
        user_id
    );
    let mut stranger = Browser::new().await?;
    let res = stranger
        .sign_in(&onboarding, &identity("o-1", &email))
        .await?;
    assert_eq!(res.status().as_u16(), 403);
    Ok(())
}

#[tokio::test]
async fn test_deleting_a_provider_or_user_removes_its_links() -> TestResult {
    let mock = start_mock_upstream().await?;
    let kept = create_provider(&mock, false).await?;
    let removed = create_provider(&mock, false).await?;
    let email = format!("{}@links.test", unique("cascade"));
    let user_id = create_password_user(&email).await?;

    let mut browser = Browser::new().await?;
    browser.password_login(&email).await?;
    expect_status(browser.link(&kept, &identity("k-1", &email)).await?, 204).await?;
    expect_status(browser.link(&removed, &identity("r-1", &email)).await?, 204).await?;

    let admin = get_auth_headers().await?;
    let res = Client::new()
        .delete(format!("{}/providers/{removed}", get_backend_url()))
        .headers(admin.clone())
        .send()
        .await?;
    expect_status(res, 200).await?;
    let links = browser.links().await?;
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["provider_id"], kept.as_str());

    let res = Client::new()
        .delete(format!("{}/users/{user_id}", get_backend_url()))
        .headers(admin)
        .send()
        .await?;
    expect_status(res, 204).await?;
    let mut stranger = Browser::new().await?;
    let res = stranger.sign_in(&kept, &identity("k-1", &email)).await?;
    assert_eq!(res.status().as_u16(), 404);
    Ok(())
}
