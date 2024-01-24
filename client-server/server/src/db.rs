//! All database related stuff.
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tokio::sync::Mutex;

use cli_ser::{cli, Data};

#[derive(Clone, Debug, sqlx::FromRow)]
pub(crate) struct User {
    username: String,
    password: String,
}
impl From<cli::Credentials> for User {
    fn from(value: cli::Credentials) -> Self {
        User {
            username: value.user.to_string(),
            password: value.password,
        }
    }
}

/// User table, since the username is not the primary key, it can be changed later.
const CREATE_USERS: &str = r#"
CREATE TABLE IF NOT EXISTS "users" (
  "id" bigserial PRIMARY KEY,
  "username" text NOT NULL,
  "password" text NOT NULL
);
"#;

const CREATE_MESSAGES: &str = r#"
CREATE TABLE IF NOT EXISTS "messages" (
  "id" bigserial PRIMARY KEY,
  "from_user_id" bigint NOT NULL,
  "text_id" bigint,
  "file_id" bigint,
  "img_id" bigint,
  "arrived" timestamp with time zone NOT NULL,
  check(
    (
      ("text_id" IS NOT NULL)::integer +
      ("file_id" IS NOT NULL)::integer +
      ("img_id" IS NOT NULL)::integer
    ) = 1
  )
);
"#;
const CREATE_CHATS: &str = r#"
CREATE TABLE IF NOT EXISTS "chats" (
  "id" bigserial PRIMARY KEY,
  "msg_id" bigint NOT NULL,
  "to_user_id" bigint NOT NULL,
  "when_recv" timestamp
);
"#;
const CREATE_TEXTS: &str = r#"
CREATE TABLE IF NOT EXISTS "texts" (
  "id" bigserial PRIMARY KEY,
  "text" text
);
"#;
const CREATE_FILES: &str = r#"
CREATE TABLE IF NOT EXISTS "files" (
  "id" bigserial PRIMARY KEY,
  "name" text,
  "bytes" bytea
);
"#;
// TODO: img format
const CREATE_IMAGES: &str = r#"
CREATE TABLE IF NOT EXISTS "images" (
  "id" bigserial PRIMARY KEY,
  "bytes" bytea
);
"#;
const ALTER_MESSAGES_USERS: &str = r#"
ALTER TABLE "messages" ADD FOREIGN KEY ("from_user_id") REFERENCES "users" ("id");
"#;
const ALTER_MESSAGES_TEXTS: &str = r#"
ALTER TABLE "messages" ADD FOREIGN KEY ("text_id") REFERENCES "texts" ("id");
"#;
const ALTER_MESSAGES_FILES: &str = r#"
ALTER TABLE "messages" ADD FOREIGN KEY ("file_id") REFERENCES "files" ("id");
"#;
const ALTER_MESSAGES_IMAGES: &str = r#"
ALTER TABLE "messages" ADD FOREIGN KEY ("img_id") REFERENCES "images" ("id");
"#;
const ALTER_CHATS_MESSAGES: &str = r#"
ALTER TABLE "chats" ADD FOREIGN KEY ("msg_id") REFERENCES "messages" ("id");
"#;
const ALTER_CHATS_USERS: &str = r#"
ALTER TABLE "chats" ADD FOREIGN KEY ("to_user_id") REFERENCES "users" ("id");
"#;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Wrong password for user `{0}`")]
    WrongPassword(String),
    #[error("User `{0}` does not exist")]
    UserDoesNotExist(String),
    #[error("Username `{0}` is already taken")]
    UsernameTaken(String),
    #[error("Inner database fail, contact the implementer!")]
    Database(sqlx::Error),
    #[error("Fail during password check, contact the implementer!")]
    Security(argon2::password_hash::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// Database handle.
///
/// ## Why tokio::Mutex
///
/// During user signing up (inserting to the database),
/// there can not be interruption between the check if exist and insert,
/// otherwise two users with the same name can be created at the same time.
///
/// std mutex is preferred over tokio mutex even in asynchronous settings...
/// however sqlx::query needs .await, so we need tokio mutex to be held
/// for the whole the select and insert.
///
/// Besides ["The primary use case for the async mutex is to provide shared mutable access to IO resources such as a database connection."](https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html).
///
/// In order to get rid of the tokio mutex there is a posibility to refactor the database with actor model.
///
/// ## Argon2
///
/// Currently a default argon2 is created for every log-in and sign-up.
/// The struct has lifetime (of the secret key) which makes it complicated for
/// tasks etc.
/// If this would be a problem (performance), the actor model would solve it.
pub(crate) struct Database {
    pool: Mutex<PgPool>,
}
impl Database {
    /// Connects to database specified by `url` and creates tables.
    ///
    /// The `url` specification can be read [here](https://docs.rs/sqlx/latest/sqlx/trait.ConnectOptions.html#implementors).
    pub(crate) async fn try_new(url: &str) -> sqlx::Result<Database> {
        let pool = PgPoolOptions::new().max_connections(5).connect(url).await?;
        sqlx::query(CREATE_USERS).execute(&pool).await?;
        sqlx::query(CREATE_MESSAGES).execute(&pool).await?;
        sqlx::query(CREATE_CHATS).execute(&pool).await?;
        sqlx::query(CREATE_TEXTS).execute(&pool).await?;
        sqlx::query(CREATE_FILES).execute(&pool).await?;
        sqlx::query(CREATE_IMAGES).execute(&pool).await?;
        sqlx::query(ALTER_MESSAGES_USERS).execute(&pool).await?;
        sqlx::query(ALTER_MESSAGES_TEXTS).execute(&pool).await?;
        sqlx::query(ALTER_MESSAGES_FILES).execute(&pool).await?;
        sqlx::query(ALTER_MESSAGES_IMAGES).execute(&pool).await?;
        sqlx::query(ALTER_CHATS_MESSAGES).execute(&pool).await?;
        sqlx::query(ALTER_CHATS_USERS).execute(&pool).await?;
        Ok(Database {
            pool: Mutex::new(pool),
        })
    }

    /// Queries user by username.
    async fn query_user(pool: &PgPool, username: &str) -> Result<Option<User>> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE username = $1")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(Error::Database)
    }

    pub(crate) async fn log_in(&self, user: impl Into<User>) -> Result<()> {
        let User { username, password } = user.into();
        let user_db = {
            let pool = self.pool.lock().await;
            Self::query_user(&pool, &username)
                .await?
                .ok_or_else(|| Error::UserDoesNotExist(username.clone()))?
        };
        Argon2::default()
            .verify_password(
                password.as_bytes(),
                &PasswordHash::new(&user_db.password).map_err(Error::Security)?,
            )
            .map_err(|_| Error::WrongPassword(username))
    }

    pub(crate) async fn sign_up(&self, user: impl Into<User>) -> Result<()> {
        let User { username, password } = user.into();
        let password = Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .map_err(Error::Security)?
            .to_string();

        let pool = self.pool.lock().await;
        if Self::query_user(&pool, &username).await?.is_some() {
            return Err(Error::UsernameTaken(username));
        }
        sqlx::query("INSERT INTO users (username, password) VALUES ($1, $2);")
            .bind(username.clone())
            .bind(password)
            .execute(&*pool)
            .await
            .map(|_| ())
            .map_err(Error::Database)
    }

    /// Records information to the database about the `data` send to all users by the `user`.
    pub(crate) async fn record_msg_to_all(&self, user: cli_ser::User, data: Data) -> Result<()> {
        let insert_data_and_msg = |insert_data, data_type| {
            format!(
                "\
WITH
  usr as (
    SELECT id FROM users WHERE username = ($1)
  ),
  data as (
    {insert_data} RETURNING id
  )
INSERT INTO messages (from_user_id, {data_type}, arrived)
SELECT usr.id, data.id, current_timestamp FROM usr, data;"
            )
        };
        let username = String::from(user);
        let pool = self.pool.lock().await;
        match data {
            Data::Text(text) => {
                sqlx::query(&insert_data_and_msg(
                    "INSERT INTO texts (text) VALUES ($2)",
                    "text_id",
                ))
                .bind(username)
                .bind(text)
                .execute(&*pool)
                .await
            }
            Data::File(file) => {
                let (name, bytes): (String, Vec<u8>) = file.into();
                sqlx::query(&insert_data_and_msg(
                    "INSERT INTO files (name, bytes) VALUES ($2, $3)",
                    "file_id",
                ))
                .bind(username)
                .bind(name)
                .bind(bytes)
                .execute(&*pool)
                .await
            }
            Data::Image(img) => {
                let bytes: Vec<u8> = img.into();
                sqlx::query(&insert_data_and_msg(
                    "INSERT INTO images (bytes) VALUES ($2)",
                    "img_id",
                ))
                .bind(username)
                .bind(bytes)
                .execute(&*pool)
                .await
            }
        }
        .map(|_| ())
        .map_err(Error::Database)
    }
}
