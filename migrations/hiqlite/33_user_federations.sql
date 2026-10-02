CREATE TABLE user_federations
(
    user_id        TEXT    NOT NULL
        CONSTRAINT user_federations_users_id_fk
            REFERENCES users
            ON UPDATE CASCADE ON DELETE CASCADE,
    provider_id    TEXT    NOT NULL
        CONSTRAINT user_federations_auth_providers_id_fk
            REFERENCES auth_providers
            ON UPDATE CASCADE ON DELETE CASCADE,
    federation_uid TEXT    NOT NULL,
    created        INTEGER NOT NULL,
    CONSTRAINT user_federations_pk
        PRIMARY KEY (provider_id, federation_uid),
    CONSTRAINT user_federations_user_provider_uk
        UNIQUE (user_id, provider_id)
) STRICT;

INSERT INTO user_federations (user_id, provider_id, federation_uid, created)
SELECT id, auth_provider_id, federation_uid, created_at
FROM users
WHERE auth_provider_id IS NOT NULL
  AND federation_uid IS NOT NULL;
