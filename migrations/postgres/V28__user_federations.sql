create table user_federations
(
    user_id        varchar not null
        constraint user_federations_users_id_fk
            references users
            on update cascade on delete cascade,
    provider_id    varchar not null
        constraint user_federations_auth_providers_id_fk
            references auth_providers
            on update cascade on delete cascade,
    federation_uid varchar not null,
    created        bigint  not null,
    constraint user_federations_pk
        primary key (provider_id, federation_uid),
    constraint user_federations_user_provider_uk
        unique (user_id, provider_id)
);

insert into user_federations (user_id, provider_id, federation_uid, created)
select id, auth_provider_id, federation_uid, created_at
from users
where auth_provider_id is not null
  and federation_uid is not null;
