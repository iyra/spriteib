use couch_rs::database::Database;
use couch_rs::types::find::FindQuery;
use couch_rs::types::view::{CouchFunc, CouchViews};
use poem::{
    get, handler,
    listener::TcpListener,
    middleware::AddData,
    web::{Data, Json, Path},
    EndpointExt, IntoResponse, Route, Server,
};
use log::{debug, error, info, trace, warn};
use serde_json::{Map, Value};
use spriteib_lib::{Comment, PostBody, Thread, _comment, _thread, RedisBus};
use config::Config;

#[handler]
fn admin(Path(name): Path<String>) -> String {
    format!("hello: {}", name)
}

#[handler]
async fn get_thread(
    Path((board, thread)): Path<(String, i32)>,
    db: Data<&Database>,
) -> impl IntoResponse {
    let find_all = FindQuery::find_all();
    let docs = db.find_raw(&find_all).await.expect("Shit");
    let emptymap = Map::new();
    return format!(
        "hello: {}, thread: {}, docs: {:?}",
        board,
        thread,
        docs.get_data()
            .iter()
            .map(|x| x.as_object().unwrap_or(&emptymap))
            .collect::<Vec<_>>()
    );
}

async fn seed_data(db: Database) {
    //let mut threads = HashMap<>::new();
    for n in 1..120 {
        let mut p = Thread {
            t: _thread(),
            _id: "".to_string(),
            _rev: "".to_string(),
            board_code: if n % 2 == 0 {
                "g".to_string()
            } else {
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
        };
        let mut doc = serde_json::to_value(p).unwrap();
        match db.create(&mut doc).await {
            Ok(r) => {
                println!(
                    "Thread document was created with ID: {} and Rev: {}",
                    r.id, r.rev
                );
                for m in 1..((n as f32 / 2.0).ceil() as i32) {
                    let thread = r.id.clone();
                    let mut c = Comment {
                        t: _comment(),
                        _id: "".to_string(),
                        _rev: "".to_string(),
                        board_code: if n % 2 == 0 {
                            "g".to_string()
                        } else {
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
                    match db.create(&mut cdoc).await {
                        Ok(q) => println!(
                            "Comment document was created with ID: {} and Rev: {}",
                            q.id, q.rev
                        ),
                        Err(err) => println!("error creating document {}: {:?}", doc, err),
                    }
                }
            }
            Err(err) => println!("error creating document {}: {:?}", doc, err),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    tracing_subscriber::fmt()
        .with_env_filter("poem=trace")
        .init();

    let s = Config::builder()
        .add_source(config::File::with_name("settings"))
        .build()
        .unwrap();

    let client =
        couch_rs::Client::new(
         s.get::<String>("couch.host")
            .expect("Bad couch.host").as_str(),
         s.get::<String>("couch.username")
            .expect("Bad couch.username").as_str(),
         s.get::<String>("couch.password")
            .expect("Bad couch.password").as_str())
        .expect("Could not create couchdb client");

    let db = client
        .db(s.get::<String>("couch.spriteib-db")
            .expect("Bad couch.spriteib-db").as_str())
        .await
        .expect("Could not access spriteib db");

    let listing_db = client
        .db(s.get::<String>("couch.spriteib-listing-db")
            .expect("Bad couch.spriteib-listing-db").as_str())
        .await
        .expect("Could not access spriteib listing db");

    let mut bus = RedisBus {
        uri: s.get::<String>("redis.connection-string")
            .expect("Bad redis.connection-string"),
        connection: None,
    };

    match bus.connect().await {
        Ok(()) => info!("Redis connection established"),
        Err(e) => panic!("{}", e),
    }

    if !db.exists("_design/user").await {
        let thread_view = CouchFunc {
            map: "function (doc) { if(!doc.archived) { if (doc.t == \"thread\") { emit([doc.bc, doc._id, 0], null) } else if (doc.t == \"comment\") { emit([doc.bc, doc.parent_thread_id, doc.pid], null) } } }".to_string(),
            reduce: None,
        };

        let views = CouchViews::new("thread_view", thread_view);
        db.create_view("user", views)
            .await
            .expect("Could not create view");
    }

    if !listing_db.exists("_design/user").await {
        let board_view = CouchFunc {
            map: "function (doc) { emit([doc.bc, doc._id, 0], null) }".to_string(),
            reduce: None,
        };

        let views = CouchViews::new("thread_view", board_view);
        db.create_view("user", views)
            .await
            .expect("Could not create view");
    }

    seed_data(db.clone()).await;

    let app = Route::new()
        .at("/board/:board<[A-Za-z]+>/:thread<\\d+>", get(get_thread))
        .with(AddData::new(db))
        .with(AddData::new(listing_db));
    Server::new(TcpListener::bind(s.get::<String>("spriteib.host").expect("Bad spriteib.host")))
        .run(app)
        .await
}
