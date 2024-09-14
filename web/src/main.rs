use config::Config;
use couch_rs::{
    database::Database,
    error::CouchError,
    types::{
        find::FindQuery,
        query::QueryParams,
        view::RawViewCollection,
    },
};
use log::{
    debug,
    error,
    info,
    trace,
    warn,
};
use poem::{
    error::{
        Error,
        InternalServerError,
        NotFoundError,
        Result,
    },
    get,
    handler,
    http::StatusCode,
    listener::TcpListener,
    middleware::AddData,
    web::{
        Data,
        Json,
        Path,
    },
    EndpointExt,
    IntoResponse,
    Response,
    Route,
    Server,
};
use serde_json::{
    json,
    Map,
    Value,
};
use spriteib_lib::{
    Comment,
    DispatchError,
    PostBody,
    RedisBus,
    Thread,
    _comment,
    _thread,
    get_couch_settings,
    get_redis_settings,
    get_sprite_settings,
};
use tera::Tera;

fn get_dynamic_settings(db: &Database, board_code: Option<String>) -> bool
{
    true
}

#[handler]
async fn get_board(Path(name): Path<String>) -> String
{
    format!("hello: {}", name)
}

#[handler]
async fn get_thread(
    Path((board, thread)): Path<(String, String)>,
    db: Data<&Database>,
    tpl: Data<&Tera>,
) -> poem::error::Result<impl IntoResponse>
{
    let settings = get_dynamic_settings(&db, None);

    /* select from [board_code, thread_id, 0] to [board_code, thread_id, inf]
     * to get the OP post and its comments all in one.
     */
    let sk = json!([board, thread, 0]);
    let ek = json!([board, thread, {}]);

    let qp = QueryParams::default().start_key(sk).end_key(ek);

    let result: Result<RawViewCollection<Value, PostBody>, CouchError> =
        db.query("user", "thread_view", Some(qp)).await;

    match result
    {
        Ok(vc) =>
        {
            match vc.rows.first()
            {
                None => Err(NotFoundError.into()),
                Some(op) =>
                {
                    // create template render context
                    let mut ctx = tera::Context::new();

                    // put in OP post - guaranteed to exist
                    ctx.insert("op", &op.value);

                    // are there comments?
                    let has_comments = vc.rows.get(1);

                    match has_comments
                    {
                        // map comments to post body
                        Some(_) => ctx.insert(
                            "comments",
                            &vc.rows[1..]
                                .iter()
                                .map(|v| v.clone().value)
                                .collect::<Vec<PostBody>>(),
                        ),

                        // no comments, just pass empty list
                        None =>
                        {
                            ctx.insert("comments", &Vec::<PostBody>::new())
                        }
                    }

                    let rendered =
                        tpl.render("thread/view.tera.html", &ctx).unwrap();

                    Ok(Response::builder().body(rendered))
                }
            }
        }
        Err(e) =>
        {
            error!("{:?}", e);
            Ok(Response::builder()
                .status(poem::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(()))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error>
{
    env_logger::init();

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
        &couch_settings.password,
    )
    .expect("Could not create couchdb client");

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

    match bus.connect().await
    {
        Ok(()) => info!("Redis connection established"),
        Err(e) => panic!("{}", e),
    }

    let tera = Tera::new("web/templates/**/*").unwrap();

    let app = Route::new()
        .at(
            "/board/:board<[A-Za-z]+>/:thread<[A-Fa-f0-9]+>/",
            get(get_thread),
        )
        .with(AddData::new(db))
        .with(AddData::new(tera))
        .at("/board/:board<[A-Za-z]+>", get(get_board))
        .with(AddData::new(listing_db))
        .with(AddData::new(tera))
        .catch_error(|_: NotFoundError| async move {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body("haha")
        });

    Server::new(TcpListener::bind(sprite_settings.run_host))
        .run(app)
        .await
}
