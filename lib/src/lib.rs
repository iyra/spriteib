use serde_json::{Value, Map};
use couch_rs::document::{DocumentCollection, TypedCouchDocument};
use couch_rs::CouchDocument;
use couch_rs::types::document::{DocumentId};
use couch_rs::types::view::{CouchFunc, CouchViews};
use chrono::{DateTime, Utc};
use chrono::serde::ts_nanoseconds;
use serde::{Serialize, Deserialize};
use redis::Cmd;
use redis::aio::{MultiplexedConnection, PubSub};
use redis::{AsyncCommands, Client, RedisError};
use uuid::Uuid;
use std::net::IpAddr;

#[derive(Serialize, Deserialize, Debug)]
pub struct PostBody {
    pub name: String,
    pub comment: String,
     #[serde(with = "ts_nanoseconds")]
    pub time: DateTime<Utc>,
    pub email: String
}

pub fn _thread() -> String { "thread".to_string() }
pub fn _comment() -> String { "comment".to_string() }

#[derive(Serialize, Deserialize, CouchDocument)]
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
    pub pinned: bool
}

#[derive(Serialize, Deserialize, CouchDocument)]
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
    pub archived: bool
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    NewThread { data: NewThreadMessage, request_id: Uuid, remote_ip: IpAddr, role: Role, board_code: String },
    NewComment { data: NewCommentMessage, request_id: Uuid, remote_ip: IpAddr, role: Role, board_code: String }
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
    User
}

pub struct RedisBus {
    pub uri: String,
    pub connection: Option<MultiplexedConnection>,
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

    pub async fn publish(&mut self, channel: &str, message: &str) {
        let ps = &mut self.connection;
        match ps {
            Some(conn) => {
                match conn.publish(channel, message.to_string()).await {
                        Ok(data) => data,
                        Err(e) => {
                            println!("Error publishing");
                        } 
                    }
            },
            None => println!("No connection specified")
        };
    }
}
