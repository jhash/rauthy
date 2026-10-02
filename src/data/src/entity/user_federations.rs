use crate::database::DB;
use chrono::Utc;
use hiqlite::macros::{FromRow, params};
use rauthy_api_types::auth_providers::ProviderLinkResponse;
use rauthy_common::is_hiqlite;
use rauthy_derive::FromPgRow;
use rauthy_error::ErrorResponse;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow, FromPgRow)]
pub struct UserFederation {
    pub user_id: String,
    pub provider_id: String,
    pub federation_uid: String,
    pub created: i64,
}

impl From<UserFederation> for ProviderLinkResponse {
    fn from(value: UserFederation) -> Self {
        Self {
            provider_id: value.provider_id,
            federation_uid: value.federation_uid,
            created: value.created,
        }
    }
}

impl UserFederation {
    pub async fn insert(
        user_id: &str,
        provider_id: &str,
        federation_uid: &str,
    ) -> Result<(), ErrorResponse> {
        let sql = r#"
INSERT INTO user_federations (user_id, provider_id, federation_uid, created)
VALUES ($1, $2, $3, $4)
ON CONFLICT (provider_id, federation_uid) DO NOTHING"#;
        let now = Utc::now().timestamp();

        if is_hiqlite() {
            DB::hql()
                .execute(sql, params!(user_id, provider_id, federation_uid, now))
                .await?;
        } else {
            DB::pg_execute(sql, &[&user_id, &provider_id, &federation_uid, &now]).await?;
        }

        Ok(())
    }

    pub async fn find(
        provider_id: &str,
        federation_uid: &str,
    ) -> Result<Option<Self>, ErrorResponse> {
        let sql = "SELECT * FROM user_federations WHERE provider_id = $1 AND federation_uid = $2";
        let res = if is_hiqlite() {
            DB::hql()
                .query_as_optional(sql, params!(provider_id, federation_uid))
                .await?
        } else {
            DB::pg_query_opt(sql, &[&provider_id, &federation_uid]).await?
        };

        Ok(res)
    }

    pub async fn find_for_user(user_id: &str) -> Result<Vec<Self>, ErrorResponse> {
        let sql = "SELECT * FROM user_federations WHERE user_id = $1 ORDER BY created ASC";
        let res = if is_hiqlite() {
            DB::hql().query_as(sql, params!(user_id)).await?
        } else {
            DB::pg_query(sql, &[&user_id], 2).await?
        };

        Ok(res)
    }

    pub async fn delete(user_id: &str, provider_id: &str) -> Result<(), ErrorResponse> {
        let sql = "DELETE FROM user_federations WHERE user_id = $1 AND provider_id = $2";

        if is_hiqlite() {
            DB::hql()
                .execute(sql, params!(user_id, provider_id))
                .await?;
        } else {
            DB::pg_execute(sql, &[&user_id, &provider_id]).await?;
        }

        Ok(())
    }
}
