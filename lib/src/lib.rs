use chrono::serde::ts_nanoseconds;
use chrono::{DateTime, Utc};
use couch_rs::document::{DocumentCollection, TypedCouchDocument};
use couch_rs::types::document::DocumentId;
use couch_rs::types::view::{CouchFunc, CouchViews};
use couch_rs::CouchDocument;
use redis::aio::{MultiplexedConnection, PubSub};
use redis::{Cmd, ToRedisArgs, FromRedisValue as RV};
use redis::{AsyncCommands, Client, RedisError};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::timestamp::context::ThreadLocalContext;
use std::fmt;
use std::net::IpAddr;
use uuid::Uuid;
use config::{Config, ConfigError};
use std::ops::DerefMut;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PostBody {
    pub name: String,
    pub comment: String,
    #[serde(with = "ts_nanoseconds")]
    pub time: DateTime<Utc>,
    pub email: String,
}

pub fn _thread() -> String {
    "thread".to_string()
}
pub fn _comment() -> String {
    "comment".to_string()
}

#[derive(Serialize, Deserialize, CouchDocument, Clone)]
pub struct Thread {
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
    pub comments: Option<Vec<Comment>> // used in the listing db, ignored otherwise
}

#[derive(Serialize, Deserialize, CouchDocument, Clone)]
pub struct Comment {
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
pub enum Message {
    NewThread {
        data: NewThreadMessage,
        request_id: Uuid,
        remote_ip: IpAddr,
        role: Role,
        board_code: String,
    },
    NewComment {
        data: NewCommentMessage,
        request_id: Uuid,
        remote_ip: IpAddr,
        role: Role,
        board_code: String,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewThreadMessage {
    pub subject: String,
    pub body: PostBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct NewCommentMessage {
    pub parent_thread_id: String,
    pub body: PostBody,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Role {
    Admin,
    Mod,
    Janny,
    User,
}

#[derive(Debug)]
pub enum PostStatus {
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
pub enum DispatchError {
    NewThreadFailed,
    NewCommentFailed,
}

#[derive(Clone, Copy)]
pub struct PostSettings {
    pub thread_comment_length: i64,
    pub comment_comment_length: i64,
    pub file_size: i64,
    pub thread_replies: i64
}

pub fn get_post_settings(s: Config) -> Result<PostSettings, ConfigError> {
    let tcl = s.get_int("spriteib.max-post-length-comment")?;
    let ccl = s.get_int("spriteib.max-post-length-thread")?;
    let fs = s.get_int("spriteib.max-file-size")?;
    let tr = s.get_int("spriteib.max-thread-comments")?;

    Ok(
        PostSettings {
            thread_comment_length: tcl,
            comment_comment_length: ccl,
            file_size: fs,
            thread_replies: tr
        }
    )
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone)]
pub struct RedisBus {
    pub uri: String,
    pub connection: Option<MultiplexedConnection>,
}

pub enum BusError {
    RedisError(RedisError),
    MissingConnection
}

impl RedisBus {
    pub async fn connect(&mut self) -> Result<(), RedisError> {
        let client = redis::Client::open(self.uri.clone()).expect("db wrong");
        let connection = client.get_multiplexed_async_connection().await?;
        self.connection = Some(connection);
        Ok(())
    }

    pub async fn pubsub(&mut self) -> Result<PubSub, RedisError> {
        // since pubsub performs a multicast for all nodes in a cluster,
        // listening to a single server in the cluster is sufficient for cluster setups
        let client = Client::open(self.uri.clone())?;
        client.get_async_pubsub().await
    }

    pub async fn publish(&mut self, channel: &str, message: &str) -> Result<(), BusError> {
        let ps = &mut self.connection;
        match ps {
            Some(conn) => match conn.publish::<&str, String, String>(channel, message.to_string()).await {
                Ok(_) => Ok(()),
                Err(e) => Err(BusError::RedisError(e))
            },
            None => Err(BusError::MissingConnection)
        }
    }

    pub async fn set_key(&mut self, key: &str, value: impl ToRedisArgs) -> Result<(), BusError> {
        let ps = &mut self.connection;
        match ps {
            Some(conn) => match conn.send_packed_command(redis::cmd("SET").arg(key).arg(value)).await {
                Ok(_) => Ok(()),
                Err(e) => Err(BusError::MissingConnection)
            },
            None => Err(BusError::MissingConnection)
        }
    }
}
