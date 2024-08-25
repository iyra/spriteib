use poem::{get, handler, listener::TcpListener, web::{Path, Data, Json}, Route, Server, middleware::AddData, EndpointExt, IntoResponse};
use couch_rs::types::find::FindQuery;
use std::error::Error;
use serde_json::{Value, Map};
use couch_rs::document::{DocumentCollection, TypedCouchDocument};
use couch_rs::CouchDocument;
use couch_rs::types::document::{DocumentId};
use couch_rs::types::view::{CouchFunc, CouchViews};
use couch_rs::database::Database;
use chrono::{DateTime, Utc};
use chrono::serde::ts_milliseconds;
use serde::{Serialize, Deserialize};

const DB_HOST: &str = "http://localhost:5984";
const SPRITE_DB: &str = "spriteib";

#[derive(Serialize, Deserialize)]
pub struct PostBody {
    pub name: String,
    pub comment: String,
     #[serde(with = "ts_milliseconds")]
    pub time: DateTime<Utc>,
    pub email: String
}

fn _thread() -> String { "thread".to_string() }
fn _comment() -> String { "comment".to_string() }

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
    pub body: PostBody
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
    pub body: PostBody
}

#[handler]
fn admin(Path(name): Path<String>) -> String {
    format!("hello: {}", name)
}

#[handler]
async fn get_thread(Path((board, thread)): Path<(String, i32)>, db: Data<&Database>) -> impl IntoResponse {
    let find_all = FindQuery::find_all();
    let docs = db.find_raw(&find_all).await.expect("Shit");
    let emptymap = Map::new();
    return format!("hello: {}, thread: {}, docs: {:?}", board, thread, docs.get_data().iter().map(|x| x.as_object().unwrap_or(&emptymap)).collect::<Vec<_>>());
    
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::fmt()
        .with_env_filter("poem=trace")
        .init();

    let client = couch_rs::Client::new(DB_HOST, "admin", "pw").expect("Could not create couchdb client");
    let db = client.db(SPRITE_DB).await.expect("Could not access spriteib db");

    if !db.exists("design/user").await {
        let thread_view = CouchFunc {
            map: "function (doc) { if (doc.t == \"thread\") { emit([doc._id, 0], doc.body.time) } else if (doc.t == \"comment\") { emit([doc.parent_thread_id, doc.post_num], doc.body.time) } }".to_string(),
            reduce: None,
        };

        let views = CouchViews::new("thread_view", thread_view);
        db.create_view("user", views).await.expect("Could not create view");
    }

    let app = Route::new()
        .at("/board/:board<[A-Za-z]+>/:thread<\\d+>", get(get_thread))
        .with(AddData::new(db));
    Server::new(TcpListener::bind("0.0.0.0:3000"))
      .run(app)
      .await
}
