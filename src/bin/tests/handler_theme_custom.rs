use crate::common::get_backend_url;
use pretty_assertions::assert_eq;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_TYPE};
use std::error::Error;

mod common;

async fn get(path: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{}{path}", get_backend_url()))
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn test_custom_css_is_appended_to_every_theme() -> Result<(), Box<dyn Error>> {
    for client_id in ["rauthy", "init_client"] {
        let res = get(&format!("/theme/{client_id}/1")).await;
        assert_eq!(res.status(), 200);
        let css = res.text().await?;
        assert!(css.starts_with("body{--text:"), "{css}");
        assert!(
            css.ends_with(".custom-theme-test{font-family:Test,serif}\n"),
            "{css}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_custom_dir_files_are_served_with_their_type() -> Result<(), Box<dyn Error>> {
    let res = get("/theme_assets/test.woff2").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), "font/woff2");
    assert_eq!(res.bytes().await?.as_ref(), b"wOF2-not-a-real-font");

    let res = get("/theme_assets/custom.css").await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers().get(CONTENT_TYPE).unwrap(), "text/css");
    Ok(())
}

#[tokio::test]
async fn test_theme_assets_refuse_other_files() -> Result<(), Box<dyn Error>> {
    for path in [
        "/theme_assets/missing.woff2",
        "/theme_assets/handler_theme_custom.rs",
        "/theme_assets/..%2Fhandler_theme_custom.rs",
        "/theme_assets/..%2F..%2F..%2F..%2Fconfig-test.toml",
        "/theme_assets/.custom.css",
    ] {
        assert_eq!(get(path).await.status(), 404, "{path}");
    }
    Ok(())
}
