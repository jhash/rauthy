use crate::database::{Cache, DB};
use crate::entity::clients::Client;
use crate::rauthy_config::RauthyConfig;
use chrono::Utc;
use hiqlite::macros::params;
use rauthy_api_types::themes::ThemeRequestResponse;
use rauthy_common::compression::{compress_br, compress_gzip};
use rauthy_common::constants::BUILD_TIME;
use rauthy_common::is_hiqlite;
use rauthy_common::utils::{deserialize, serialize};
use rauthy_error::{ErrorResponse, ErrorResponseType};
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;
use tracing::{error, warn};

// used for possible future struct updates to be able to deserialize old themes properly
static LATEST_CSS_VERSION: i32 = 1;
static CACHE_KEY_EMAIL_CSS: &str = "css_rauthy_light_email";
static CUSTOM_THEME: LazyLock<Option<CustomTheme>> = LazyLock::new(CustomTheme::load);

const CUSTOM_CSS_FILE: &str = "custom.css";
const ASSET_TYPES: [(&str, &str); 8] = [
    ("css", "text/css"),
    ("woff2", "font/woff2"),
    ("woff", "font/woff"),
    ("ttf", "font/ttf"),
    ("otf", "font/otf"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("webp", "image/webp"),
];

#[derive(Debug)]
pub struct CustomTheme {
    dir: PathBuf,
    css: String,
    modified: i64,
}

impl CustomTheme {
    fn load() -> Option<Self> {
        let dir = PathBuf::from(RauthyConfig::get().vars.theme.custom_dir.as_deref()?);
        let path = dir.join(CUSTOM_CSS_FILE);
        let (css, modified) = match std::fs::read_to_string(&path) {
            Ok(css) => {
                let modified = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or_default();
                (css, modified)
            }
            Err(err) => {
                warn!("No custom theme CSS at {}: {err}", path.display());
                (String::default(), 0)
            }
        };

        Some(Self { dir, css, modified })
    }

    pub fn get() -> Option<&'static Self> {
        CUSTOM_THEME.as_ref()
    }

    fn modified() -> i64 {
        Self::get().map(|c| c.modified).unwrap_or_default()
    }

    pub fn content_type(name: &str) -> Option<&'static str> {
        let is_plain_name = !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
        if !is_plain_name {
            return None;
        }
        let (_, ext) = name.rsplit_once('.')?;
        ASSET_TYPES
            .iter()
            .find(|(e, _)| *e == ext)
            .map(|(_, content_type)| *content_type)
    }

    pub async fn read_asset(name: &str) -> Result<(Vec<u8>, &'static str), ErrorResponse> {
        let not_found = || ErrorResponse::new(ErrorResponseType::NotFound, "theme asset not found");
        let slf = Self::get().ok_or_else(not_found)?;
        let content_type = Self::content_type(name).ok_or_else(not_found)?;
        let bytes = tokio::fs::read(slf.dir.join(name))
            .await
            .map_err(|_| not_found())?;
        Ok((bytes, content_type))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThemeCssFull {
    pub client_id: String,
    // used for the etag
    pub last_update: i64,
    // Should always be 1 for now, but makes it possible to have different structs
    // in the future for proper deserialization.
    pub version: i64,
    pub light: ThemeCss,
    pub dark: ThemeCss,
    pub border_radius: String,
}

impl From<&mut hiqlite::Row<'_>> for ThemeCssFull {
    fn from(row: &mut hiqlite::Row<'_>) -> Self {
        let version: i64 = row.get("version");
        let (light, dark) = if version == 1 {
            let light = ThemeCss::from(row.get::<Vec<u8>>("light").as_slice());
            let dark = ThemeCss::from(row.get::<Vec<u8>>("dark").as_slice());
            (light, dark)
        } else {
            error!("Invalid CSS version {version} returned from database, using defaults");
            (ThemeCss::default_light(), ThemeCss::default_dark())
        };

        Self {
            client_id: row.get("client_id"),
            last_update: row.get("last_update"),
            version,
            light,
            dark,
            border_radius: row.get("border_radius"),
        }
    }
}

impl From<tokio_postgres::Row> for ThemeCssFull {
    fn from(row: tokio_postgres::Row) -> Self {
        let version: i32 = row.get("version");
        let (light, dark) = if version == 1 {
            let bytes: Vec<u8> = row.get("light");
            let light = ThemeCss::from(bytes.as_slice());
            let bytes: Vec<u8> = row.get("dark");
            let dark = ThemeCss::from(bytes.as_slice());

            (light, dark)
        } else {
            error!("Invalid CSS version {version} returned from database, using defaults");
            (ThemeCss::default_light(), ThemeCss::default_dark())
        };

        Self {
            client_id: row.get("client_id"),
            last_update: row.get("last_update"),
            version: version as i64,
            light,
            dark,
            border_radius: row.get("border_radius"),
        }
    }
}

impl Default for ThemeCssFull {
    fn default() -> Self {
        Self {
            client_id: "rauthy".to_string(),
            last_update: BUILD_TIME.timestamp(),
            version: 1,
            light: ThemeCss::default_light(),
            dark: ThemeCss::default_dark(),
            border_radius: "5px".to_string(),
        }
    }
}

impl From<ThemeCssFull> for ThemeRequestResponse {
    fn from(t: ThemeCssFull) -> Self {
        Self {
            client_id: t.client_id,
            light: rauthy_api_types::themes::ThemeCss::from(t.light),
            dark: rauthy_api_types::themes::ThemeCss::from(t.dark),
            border_radius: t.border_radius,
        }
    }
}

// CRUD
impl ThemeCssFull {
    /// Returns either the custom theme for a client, if it exists, and the default on any error.
    pub async fn find_with_default(client_id: String) -> Result<Self, ErrorResponse> {
        // TODO cache these - they are fetched for each HTML Cached Get
        let sql = "SELECT * FROM themes WHERE client_id = $1";
        let slf: Self = if is_hiqlite() {
            DB::hql()
                .query_map_optional(sql, params!(client_id))
                .await?
        } else {
            DB::pg_query_opt(sql, &[&client_id]).await?
        }
        .unwrap_or_default();

        Ok(slf)
    }

    /// Returns either the custom theme for a client, if it exists, or tries to find the `rauthy`
    /// theme as a fallback. If both values don't exist, returns default values.
    pub async fn find_with_fallback(client_id: String) -> Result<Self, ErrorResponse> {
        let is_rauthy = client_id == "rauthy";

        let sql = "SELECT * FROM themes WHERE client_id = $1";
        let opt: Option<Self> = if is_hiqlite() {
            DB::hql()
                .query_map_optional(sql, params!(client_id))
                .await?
        } else {
            DB::pg_query_opt(sql, &[&client_id]).await?
        };
        if let Some(slf) = opt {
            return Ok(slf);
        }

        let slf: Self = if !is_rauthy {
            if is_hiqlite() {
                DB::hql().query_map_optional(sql, params!("rauthy")).await?
            } else {
                DB::pg_query_opt(sql, &[&"rauthy"]).await?
            }
            .unwrap_or_default()
        } else {
            Self::default()
        };

        Ok(slf)
    }

    /// Returns the CSS variables for the light theme to be inserted directly into
    /// an E-Mail `body { }` CSS.
    pub async fn find_theme_variables_email() -> Result<String, ErrorResponse> {
        if let Some(bytes) = DB::hql()
            .get_bytes(Cache::Html, CACHE_KEY_EMAIL_CSS)
            .await?
        {
            let vars = String::from_utf8_lossy(&bytes);
            return Ok(vars.to_string());
        }

        let full = Self::find_with_fallback("rauthy".to_string()).await?;
        let mut vars = String::with_capacity(128);
        full.light.append_css(&mut vars)?;
        write!(vars, "--border-radius:{};", full.border_radius)?;

        DB::hql()
            .put_bytes(
                Cache::Html,
                CACHE_KEY_EMAIL_CSS,
                vars.as_bytes().to_vec(),
                None,
            )
            .await?;

        Ok(vars)
    }

    /// Returns `last_update` for the given `client_id`s Theme CSS. If no custom theme exists,
    /// the latest TS for the `Rauthy` theme is returned.
    #[inline(always)]
    pub async fn find_theme_ts(client_id: String) -> Result<i64, ErrorResponse> {
        let last_update = Self::find_with_default(client_id).await?.last_update;
        Ok(last_update.max(CustomTheme::modified()))
    }

    pub async fn find_for_client_uri(
        client_uri: Option<&str>,
    ) -> Result<(String, i64), ErrorResponse> {
        let client = match client_uri {
            Some(uri) => Client::find_by_client_uri(uri).await?,
            None => None,
        };
        let client_id = client.map(|c| c.id).unwrap_or_else(|| "rauthy".to_string());
        let ts = Self::find_theme_ts(client_id.clone()).await?;
        Ok((client_id, ts))
    }

    #[inline(always)]
    pub async fn find_theme_ts_rauthy() -> Result<i64, ErrorResponse> {
        Self::find_theme_ts("rauthy".to_string()).await
    }

    pub async fn delete(client_id: String) -> Result<(), ErrorResponse> {
        let id = client_id.clone();
        let sql = "DELETE FROM themes WHERE client_id = $1";

        if is_hiqlite() {
            DB::hql().execute(sql, params!(id)).await?;
        } else {
            DB::pg_execute(sql, &[&id]).await?;
        }

        DB::hql().clear_cache(Cache::ThemeTs).await?;
        DB::hql().clear_cache(Cache::Html).await?;

        Ok(())
    }

    pub async fn upsert(
        client_id: String,
        light: ThemeCss,
        dark: ThemeCss,
        border_radius: String,
    ) -> Result<(), ErrorResponse> {
        let id = client_id.clone();
        let now = Utc::now().timestamp();
        let light = light.as_bytes();
        let dark = dark.as_bytes();

        let sql = r#"
INSERT INTO themes (client_id, last_update, version, light, dark, border_radius)
VALUES ($1, $2, $3, $4, $5, $6)
ON CONFLICT (client_id) DO UPDATE
SET last_update = $2, version = $3, light = $4, dark = $5, border_radius = $6
"#;

        if is_hiqlite() {
            DB::hql()
                .execute(
                    sql,
                    params!(id, now, LATEST_CSS_VERSION, light, dark, border_radius),
                )
                .await?;
        } else {
            DB::pg_execute(
                sql,
                &[
                    &id,
                    &now,
                    &LATEST_CSS_VERSION,
                    &light,
                    &dark,
                    &border_radius,
                ],
            )
            .await?;
        }

        DB::hql().clear_cache(Cache::ThemeTs).await?;
        DB::hql().clear_cache(Cache::Html).await?;

        Ok(())
    }
}

impl ThemeCssFull {
    #[inline]
    fn cache_key_br(client_id: &str) -> String {
        format!("{client_id}_theme_br")
    }

    #[inline]
    fn cache_key_gzip(client_id: &str) -> String {
        format!("{client_id}_theme_gzip")
    }

    pub async fn plain(client_id: String) -> Result<String, ErrorResponse> {
        let slf = Self::find_with_fallback(client_id).await?;
        let mut res = slf.as_css()?;
        if let Some(custom) = CustomTheme::get() {
            res.push_str(&custom.css);
        }
        Ok(res)
    }

    pub async fn br(client_id: &str) -> Result<Vec<u8>, ErrorResponse> {
        if let Some(bytes) = DB::hql()
            .get_bytes(Cache::Html, Self::cache_key_br(client_id))
            .await?
        {
            return Ok(bytes);
        }

        let plain = Self::plain(client_id.to_string()).await?;
        let compressed = compress_br(plain.as_bytes())?;
        DB::hql()
            .put_bytes(
                Cache::Html,
                Self::cache_key_br(client_id),
                compressed.clone(),
                None,
            )
            .await?;

        Ok(compressed)
    }

    pub async fn gzip(client_id: &str) -> Result<Vec<u8>, ErrorResponse> {
        if let Some(bytes) = DB::hql()
            .get_bytes(Cache::Html, Self::cache_key_gzip(client_id))
            .await?
        {
            return Ok(bytes);
        }

        let plain = Self::plain(client_id.to_string()).await?;
        let compressed = compress_gzip(plain.as_bytes())?;
        DB::hql()
            .put_bytes(
                Cache::Html,
                Self::cache_key_gzip(client_id),
                compressed.clone(),
                None,
            )
            .await?;

        Ok(compressed)
    }

    pub fn as_css(&self) -> Result<String, ErrorResponse> {
        let mut s = String::with_capacity(1100);

        write!(s, "body{{")?;
        self.light.append_css(&mut s)?;
        write!(s, "--border-radius:{};", self.border_radius)?;
        write!(s, "}}")?;

        write!(s, ".theme-dark{{")?;
        self.dark.append_css(&mut s)?;
        write!(s, "}}")?;

        write!(s, "@media (prefers-color-scheme: dark){{")?;
        write!(s, "body{{")?;
        self.dark.append_css(&mut s)?;
        write!(s, "}}")?;

        write!(s, ".theme-light{{")?;
        self.light.append_css(&mut s)?;
        write!(s, "}}")?;
        write!(s, "}}")?;

        Ok(s)
    }

    #[inline]
    pub fn as_css_light(&self) -> Result<String, ErrorResponse> {
        let mut s = String::with_capacity(280);

        write!(s, "body{{")?;
        self.light.append_css(&mut s)?;
        write!(s, "--border-radius:{};", self.border_radius)?;
        write!(s, "}}")?;

        Ok(s)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ThemeCss {
    // HSL values without prefix
    pub text: [u16; 3],
    pub text_high: [u16; 3],
    pub bg: [u16; 3],
    pub bg_high: [u16; 3],
    pub action: [u16; 3],
    pub accent: [u16; 3],
    pub error: [u16; 3],

    // fully valid values
    pub btn_text: String,
    pub theme_sun: String,
    pub theme_moon: String,
}

impl From<rauthy_api_types::themes::ThemeCss> for ThemeCss {
    fn from(value: rauthy_api_types::themes::ThemeCss) -> Self {
        Self {
            text: value.text,
            text_high: value.text_high,
            bg: value.bg,
            bg_high: value.bg_high,
            action: value.action,
            accent: value.accent,
            error: value.error,
            btn_text: value.btn_text,
            theme_sun: value.theme_sun,
            theme_moon: value.theme_moon,
        }
    }
}

impl From<ThemeCss> for rauthy_api_types::themes::ThemeCss {
    fn from(t: ThemeCss) -> Self {
        Self {
            text: t.text,
            text_high: t.text_high,
            bg: t.bg,
            bg_high: t.bg_high,
            action: t.action,
            accent: t.accent,
            error: t.error,
            btn_text: t.btn_text,
            theme_sun: t.theme_sun,
            theme_moon: t.theme_moon,
        }
    }
}

impl From<&[u8]> for ThemeCss {
    fn from(value: &[u8]) -> Self {
        deserialize(value).unwrap()
    }
}

impl ThemeCss {
    pub fn append_css(&self, s: &mut String) -> Result<(), ErrorResponse> {
        write!(
            s,
            "--text:{} {} {};",
            self.text[0], self.text[1], self.text[2]
        )?;
        write!(
            s,
            "--text-high:{} {} {};",
            self.text_high[0], self.text_high[1], self.text_high[2]
        )?;
        write!(s, "--bg:{} {} {};", self.bg[0], self.bg[1], self.bg[2])?;
        write!(
            s,
            "--bg-high:{} {} {};",
            self.bg_high[0], self.bg_high[1], self.bg_high[2]
        )?;
        write!(
            s,
            "--action:{} {} {};",
            self.action[0], self.action[1], self.action[2]
        )?;
        write!(
            s,
            "--accent:{} {} {};",
            self.accent[0], self.accent[1], self.accent[2]
        )?;
        write!(
            s,
            "--error:{} {} {};",
            self.error[0], self.error[1], self.error[2]
        )?;

        write!(s, "--btn-text:{};", self.btn_text)?;
        write!(s, "--theme-sun:{};", self.theme_sun)?;
        write!(s, "--theme-moon:{};", self.theme_moon)?;

        Ok(())
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        serialize(self).unwrap()
    }

    pub(crate) fn default_dark() -> Self {
        Self {
            text: [34, 5, 75],
            text_high: [34, 7, 90],
            bg: [200, 40, 6],
            bg_high: [200, 20, 17],
            action: [34, 100, 59],
            accent: [265, 100, 53],
            error: [15, 100, 37],
            btn_text: "hsl(var(--bg))".to_string(),
            theme_sun: "hsla(var(--action) / .7)".to_string(),
            theme_moon: "hsla(var(--accent) / .85)".to_string(),
        }
    }

    pub(crate) fn default_light() -> Self {
        Self {
            text: [200, 5, 37],
            text_high: [200, 15, 25],
            bg: [34, 25, 97],
            bg_high: [34, 20, 90],
            action: [34, 100, 40],
            accent: [265, 100, 53],
            error: [15, 100, 37],
            btn_text: "white".to_string(),
            theme_sun: "hsla(var(--action) / .7)".to_string(),
            theme_moon: "hsla(var(--accent) / .85)".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_css_fmt() {
        let theme = ThemeCssFull::default();
        theme.as_css().unwrap();
    }

    #[test]
    fn test_custom_asset_content_types() {
        assert_eq!(
            CustomTheme::content_type("Lora-Variable.woff2"),
            Some("font/woff2")
        );
        assert_eq!(CustomTheme::content_type("custom.css"), Some("text/css"));
        assert_eq!(CustomTheme::content_type("logo.svg"), Some("image/svg+xml"));
        assert_eq!(CustomTheme::content_type("script.js"), None);
        assert_eq!(CustomTheme::content_type("../secrets.toml"), None);
        assert_eq!(CustomTheme::content_type("..woff2"), None);
        assert_eq!(CustomTheme::content_type(".hidden.css"), None);
        assert_eq!(CustomTheme::content_type("a/b.css"), None);
        assert_eq!(CustomTheme::content_type("noext"), None);
    }
}
