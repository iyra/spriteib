use config::Config;
use couch_rs::database::Database;
use couch_rs::types::find::FindQuery;
use couch_rs::types::view::{CouchFunc, CouchViews, RawViewCollection};
use couch_rs::types::query::QueryParams;
use couch_rs::error::CouchError;
use log::{debug, error, info, trace, warn};
use poem::{
    get, handler,
    listener::TcpListener,
    middleware::AddData,
    web::{Data, Json, Path},
    EndpointExt, IntoResponse, Route, Server,
    error::{Error, Result, NotFoundError, InternalServerError}, Response
};
use poem::http::StatusCode;
use serde_json::{Map, Value,json};
use serde_json::value::Value::Array;
use spriteib_lib::{Comment, DispatchError, PostBody, RedisBus, Thread, _comment, _thread,
get_sprite_settings, get_redis_settings, get_couch_settings};
use tera::Tera;

fn get_dynamic_settings(db: &Database, board_code: Option<String>) ->  bool {
    true
}

#[handler]
fn admin(Path(name): Path<String>) -> String {
    format!("hello: {}", name)
}

#[handler]
async fn get_thread(
    Path((board, thread)): Path<(String, String)>,
    db: Data<&Database>,
    tpl: Data<&Tera>
) -> poem::error::Result<impl IntoResponse> {
    let settings = get_dynamic_settings(&db, None);

    /* select from [board_code, thread_id, 0] to [board_code, thread_id, inf] to
     * get the OP post and its comments all in one.
     */
    let sk = json!([board, thread, 0]);
    let ek = json!([board, thread, {}]);

    let qp = QueryParams::default()
        .start_key(sk)
        .end_key(ek);

    let result: Result<RawViewCollection<Value, PostBody>, CouchError> = db.query("user", "thread_view", Some(qp)).await;

    match result {
        Ok(vc) => {
            match vc.rows.first() {
                None => Err(NotFoundError.into()),
                Some(op) => {

                    // create template render context
                    let mut ctx = tera::Context::new();

                    // put in OP post - guaranteed to exist
                    ctx.insert("op", &op.value);

                    // are there comments?
                    let has_comments = vc.rows.get(1);

                    match has_comments {
                        // map comments to post body
                        Some(_) => ctx.insert("comments",
                            &vc.rows[1..]
                                .iter()
                                .map(|v| v.clone().value)
                                .collect::<Vec<PostBody>>()),

                        // no comments, just pass empty list
                        None => ctx.insert("comments",
                            &Vec::<PostBody>::new())
                    }

                    let rendered = tpl.render("thread/view.tera.html", &ctx).unwrap();
                    Ok(Response::builder().body(rendered))
                }
            }
        },
        Err(e) => {
            error!("{:?}", e);
            Ok(Response::builder()
                .status(poem::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(()))
        }
    }
}

async fn seed_data(db: Database) {
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
            comments: None
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
    env_logger::init();
    /*tracing_subscriber::fmt()
        .with_env_filter("poem=trace")
        .init();*/

    let s = Config::builder()
        .add_source(config::File::with_name("settings"))
        .build()
        .unwrap();

    let sprite_settings = get_sprite_settings(&s).unwrap();
    let couch_settings = get_couch_settings(&s).unwrap();
    let redis_settings = get_redis_settings(&s).unwrap();

    let client = couch_rs::Client::new(
        &couch_settings.host,
        &couch_settings.username,
        &couch_settings.password
    ).expect("Could not create couchdb client");

    let db = client
        .db(&couch_settings.db_spriteib)
        .await
        .expect("Could not access spriteib db");

    let listing_db = client
        .db(&couch_settings.db_listing)
        .await
        .expect("Could not access spriteib listing db");

    let mut bus = RedisBus {
        uri: redis_settings.connection_string,
        connection: None,
    };

    match bus.connect().await {
        Ok(()) => info!("Redis connection established"),
        Err(e) => panic!("{}", e),
    }

    if !db.exists("_design/user").await {
        let thread_view = CouchFunc {
            map: "function (doc) {
                        if (doc.t == \"thread\" && !doc.archived) {
                            emit([doc.bc, doc._id, 0], doc.body)
                        }
                        else if (doc.t == \"comment\") {
                            emit([doc.bc, doc.parent_thread_id, doc.pid], doc.body)
                        }
                    }".to_string(),
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
        listing_db.create_view("user", views)
            .await
            .expect("Could not create view");
    }

    let mut tera = Tera::new("web/templates/**/*").unwrap();

    //seed_data(db.clone()).await;

    let app = Route::new()
        .at("/board/:board<[A-Za-z]+>/:thread<[A-Fa-f0-9]+>", get(get_thread))
        .with(AddData::new(db))
        .with(AddData::new(tera))
        .catch_error(|_: NotFoundError| async move {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body("haha")
            });
        //.with(AddData::new(listing_db));
    Server::new(TcpListener::bind(
        sprite_settings.run_host
    ))
    .run(app)
    .await
}
