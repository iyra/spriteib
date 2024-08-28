use couch_rs::database::Database;
use futures_util::StreamExt as _;
use log::{debug, error, info, trace, warn};
use spriteib_lib::{Comment, Message, PostBody, RedisBus, Role, Thread, _comment, _thread};
use std::net::IpAddr;
use uuid::Uuid;
use config::Config;
use std::collections::HashMap;

async fn dispatch_message(message: &Message) {
    match message {
        Message::NewThread {
            data,
            request_id,
            remote_ip,
            board_code,
            role,
        } => debug!("new thread"),
        Message::NewComment {
            data,
            request_id,
            remote_ip,
            board_code,
            role,
        } => debug!("new comment"),
    }
}

async fn new_thread(
    db: Database,
    pb: PostBody,
    rid: Uuid,
    rip: IpAddr,
    boad_code: &str,
    role: Role,
) {
    let p = Thread {
        t: _thread(),
        _id: "".to_string(),
        _rev: "".to_string(),
        board_code: if 20 % 2 == 0 {
            "g".to_string()
        } else {
            "b".to_string()
        },
        thread_num: 20,
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
        Ok(_) => info!("Thread created"),
        Err(err) => println!("error creating document {}: {:?}", doc, err),
    }
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    env_logger::init();
    let s = Config::builder()
        .add_source(config::File::with_name("settings"))
        .build()
        .unwrap();

    const channels: &[&str] = &["NewThread", "NewComment"];

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

    let mut ps = match bus.pubsub().await {
        Ok(p) => {
            info!("Redis pub/sub client obtained");
            p
        }
        Err(e) => panic!("{}", e),
    };

    for c in channels {
        match ps.subscribe(c).await {
            Ok(()) => info!("Redis subscription to {}", c),
            Err(e) => error!("{}", e),
        }
    }

    while let Some(msg) = ps.on_message().next().await {
        tokio::task::spawn(async move {
            // Parse the payload
            let payload = msg.get_payload::<String>().unwrap();
            match serde_json::from_str::<Message>(&payload) {
                Ok(m) => dispatch_message(&m).await,
                Err(e) => warn!("Could not deserialize '{}': {}", &payload, e),
            }
        });
    }

    bus.publish("x", "{\"h\": 3}").await;
    Ok(())
}
