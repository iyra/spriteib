use std::{
    fmt,
    net::IpAddr,
    ops::DerefMut,
};

use chrono::{
    serde::ts_nanoseconds,
    DateTime,
    Utc,
};
use config::{
    Config,
    ConfigError,
};
use couch_rs::{
    database::Database,
    document::{
        DocumentCollection,
        TypedCouchDocument,
    },
    types::{
        document::DocumentId,
        view::{
            CouchFunc,
            CouchViews,
        },
    },
    CouchDocument,
};
use log::{
    debug,
    error,
    info,
    trace,
    warn,
};
use redis::{
    aio::{
        MultiplexedConnection,
        PubSub,
    },
    AsyncCommands,
    Client,
    Cmd,
    FromRedisValue as RV,
    RedisError,
    ToRedisArgs,
};
use serde::{
    Deserialize,
    Serialize,
};
use serde_json::{
    Map,
    Value,
};
use uuid::{
    timestamp::context::ThreadLocalContext,
    Uuid,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostBody
{
    pub name: String,
    pub comment: String,
    #[serde(with = "ts_nanoseconds")]
    pub time: DateTime<Utc>,
    pub email: String,
}

pub fn _thread() -> String
{
    "thread".to_string()
}
pub fn _comment() -> String
{
    "comment".to_string()
}

#[derive(Serialize, Deserialize, CouchDocument, Clone)]
pub struct Thread
{
    #[serde(default = "_thread")]
    pub t: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub _id: DocumentId,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub _rev: String,
    #[serde(rename = "bc")]
    pub board_code: String,
    #[serde(rename = "tid")]
    pub thread_num: i32,
    pub body: PostBody,
    #[serde(with = "ts_nanoseconds")]
    pub bump_time: DateTime<Utc>,
    pub archived: bool,
    pub pinned: bool,
    pub comments: Option<Vec<Comment>>, /* used in the listing db, ignored
                                         * otherwise */
}

#[derive(Serialize, Deserialize, CouchDocument, Clone)]
pub struct Comment
{
    #[serde(default = "_comment")]
    pub t: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub _id: DocumentId,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub _rev: String,
    #[serde(rename = "bc")]
    pub board_code: String,
    #[serde(rename = "pid")]
    pub post_num: i32,
    pub parent_thread_id: DocumentId,
    pub body: PostBody,
    pub archived: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Message
{
    NewThread
    {
        data: NewThreadMessage,
        request_id: Uuid,
        remote_ip: IpAddr,
        role: Role,
        board_code: String,
    },
    NewComment
    {
        data: NewCommentMessage,
        request_id: Uuid,
        remote_ip: IpAddr,
        role: Role,
        board_code: String,
    },
    PruneThreads
    {
        all_boards: bool,
        board_code: Option<String>,
    },
    PublishRss
    {
        all_boards: bool,
        board_code: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewThreadMessage
{
    pub subject: String,
    pub body: PostBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewCommentMessage
{
    pub parent_thread_id: String,
    pub body: PostBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Role
{
    Admin,
    Mod,
    Janny,
    User,
}

#[derive(Debug)]
pub enum PostStatus
{
    BannedIp,
    TooFast,
    ThreadLocked,
    BoardLocked,
    BannedWord,
    BannedName,
    BannedEmail,
    ThreadArchived,
    LargeThread,
    LargeName,
    LargeComment,
    LargeEmail,
    LargeFile,
    DuplicateFile,
    BadMIME,
    FailedProcessing,
    Ok,
}

#[derive(Debug)]
pub enum DispatchError
{
    NewThreadFailed,
    NewThreadCreatedWithError,
    NewCommentFailed,
}

#[derive(Clone)]
pub struct SpriteSettings
{
    pub run_host: String,
    pub post_op_max_length: i64,
    pub post_op_max_file_size: i64,
    pub post_comment_max_length: i64,
    pub post_comment_max_file_size: i64,
    pub thread_max_comments: i64,
    pub board_max_threads: i64,
}

pub struct CouchSettings
{
    pub host: String,
    pub username: String,
    pub password: String,
    pub db_spriteib: String,
    pub db_listing: String,
}

pub struct RedisSettings
{
    pub connection_string: String,
}

pub fn get_redis_settings(s: &Config) -> Result<RedisSettings, ConfigError>
{
    let cs = s.get_string("redis.connection-string")?;
    Ok(RedisSettings {
        connection_string: cs,
    })
}

pub fn get_couch_settings(s: &Config) -> Result<CouchSettings, ConfigError>
{
    let h = s.get_string("couch.host")?;
    let u = s.get_string("couch.username")?;
    let p = s.get_string("couch.password")?;
    let dbs = s.get_string("couch.db.spriteib")?;
    let dbl = s.get_string("couch.db.listing")?;

    Ok(CouchSettings {
        host: h,
        username: u,
        password: p,
        db_spriteib: dbs,
        db_listing: dbl,
    })
}

pub fn get_sprite_settings(s: &Config) -> Result<SpriteSettings, ConfigError>
{
    let rh = s.get_string("spriteib.run.host")?;
    let poml = s.get_int("spriteib.post.op.max-length")?;
    let pomfs = s.get_int("spriteib.post.op.max-file-size")?;
    let pcml = s.get_int("spriteib.post.comment.max-length")?;
    let pcmfs = s.get_int("spriteib.post.comment.max-file-size")?;
    let tmc = s.get_int("spriteib.thread.max-comments")?;
    let bmt = s.get_int("spriteib.board.max-threads")?;

    Ok(SpriteSettings {
        run_host: rh,
        post_op_max_length: poml,
        post_op_max_file_size: pomfs,
        post_comment_max_length: pcml,
        post_comment_max_file_size: pcmfs,
        thread_max_comments: tmc,
        board_max_threads: bmt,
    })
}

impl fmt::Display for PostStatus
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result
    {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for DispatchError
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result
    {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub struct RedisBus
{
    pub uri: String,
    pub connection: Option<MultiplexedConnection>,
}

#[derive(Debug)]
pub enum BusError
{
    RedisError(RedisError),
    MissingConnection,
}

impl RedisBus
{
    pub async fn connect(&mut self) -> Result<(), RedisError>
    {
        let client = redis::Client::open(self.uri.clone()).expect("db wrong");
        let connection = client.get_multiplexed_async_connection().await?;
        self.connection = Some(connection);
        Ok(())
    }

    pub async fn pubsub(&mut self) -> Result<PubSub, RedisError>
    {
        // since pubsub performs a multicast for all nodes in a cluster,
        // listening to a single server in the cluster is sufficient for cluster setups
        let client = Client::open(self.uri.clone())?;
        client.get_async_pubsub().await
    }

    pub async fn publish(
        &mut self,
        channel: &str,
        message: &str,
    ) -> Result<(), BusError>
    {
        let ps = &mut self.connection;
        match ps
        {
            Some(conn) => match conn
                .publish::<&str, String, String>(channel, message.to_string())
                .await
            {
                Ok(_) => Ok(()),
                Err(e) => Err(BusError::RedisError(e)),
            },
            None => Err(BusError::MissingConnection),
        }
    }

    pub async fn set_key(
        &mut self,
        key: &str,
        value: impl ToRedisArgs,
        expiry: i32,
    ) -> Result<(), BusError>
    {
        let ps = &mut self.connection;
        match ps
        {
            Some(conn) =>
            {
                let mut base_cmd = redis::cmd("SET");
                let mut cmd = base_cmd.arg(key).arg(value);
                if expiry > 0
                {
                    cmd = cmd.arg("EX").arg(expiry);
                }
                match conn.send_packed_command(cmd).await
                {
                    Ok(_) => Ok(()),
                    Err(e) => Err(BusError::RedisError(e)),
                }
            }
            None => Err(BusError::MissingConnection),
        }
    }

    pub async fn set_status(
        &mut self,
        request_id: String,
        message: String,
        duration: i32,
    ) -> Result<(), BusError>
    {
        self.set_key(&request_id, message, duration).await
    }
}

async fn seed_data(db: Database)
{
    for n in 1..120
    {
        let p = Thread {
            t: _thread(),
            _id: "".to_string(),
            _rev: "".to_string(),
            board_code: if n % 2 == 0
            {
                "g".to_string()
            }
            else
            {
                "b".to_string()
            },
            thread_num: n,
            body: PostBody {
                name: "test".to_string(),
                comment: "x".to_string(),
                time: chrono::offset::Utc::now(),
                email: "x@y.com".to_string(),
            },
            bump_time: chrono::offset::Utc::now(),
            archived: false,
            pinned: false,
            comments: None,
        };
        let mut doc = serde_json::to_value(p).unwrap();
        match db.create(&mut doc).await
        {
            Ok(r) =>
            {
                println!(
                    "Thread document was created with ID: {} and Rev: {}",
                    r.id, r.rev
                );
                for m in 1..((n as f32 / 2.0).ceil() as i32)
                {
                    let thread = r.id.clone();
                    let mut c = Comment {
                        t: _comment(),
                        _id: "".to_string(),
                        _rev: "".to_string(),
                        board_code: if n % 2 == 0
                        {
                            "g".to_string()
                        }
                        else
                        {
                            "b".to_string()
                        },
                        post_num: m,
                        body: PostBody {
                            name: "test".to_string(),
                            comment: "x".to_string(),
                            time: chrono::offset::Utc::now(),
                            email: "x@y.com".to_string(),
                        },
                        parent_thread_id: thread,
                        archived: false,
                    };
                    let mut cdoc = serde_json::to_value(c).unwrap();
                    match db.create(&mut cdoc).await
                    {
                        Ok(q) => println!(
                            "Comment document was created with ID: {} and \
                             Rev: {}",
                            q.id, q.rev
                        ),
                        Err(err) => println!(
                            "error creating document {}: {:?}",
                            doc, err
                        ),
                    }
                }
            }
            Err(err) => println!("error creating document {}: {:?}", doc, err),
        }
    }
}
