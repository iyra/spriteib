use serde_json::{Value, Map};
use couch_rs::document::{DocumentCollection, TypedCouchDocument};
use couch_rs::CouchDocument;
use couch_rs::types::document::{DocumentId};
use couch_rs::types::view::{CouchFunc, CouchViews};
use couch_rs::database::Database;
use chrono::{DateTime, Utc};
use chrono::serde::ts_nanoseconds;
use serde::{Serialize, Deserialize};
#[derive(Serialize, Deserialize)]
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

pub struct Message {};

#[derive(Serialize, Deserialize)]
pub struct NewThreadMessage {
    pub subject: String,
    pub post: NewPost,
    pub request_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct NewPost {
    pub remote_ip: String,
    pub board_code: String,
    pub body: PostBody
}

#[derive(Serialize, Deserialize)]
pub struct NewCommentMessage {
    pub parent_thread_id: String,
    pub body: PostBody,
    pub request_id: String,
}
