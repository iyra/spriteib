use config::Config;
use couch_rs::database::Database;
use futures_util::StreamExt as _;
use log::{debug, error, info, trace, warn};
use spriteib_lib::{
    Comment, DispatchError, Message, PostBody, RedisBus, Role, Thread, _comment, _thread,
    PostStatus, get_post_settings, PostSettings, BusError
};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;
use std::collections::HashMap;

async fn dispatch_message(
    message: &Message,
    db: &Database,
    listing_db: &Database,
    post_settings: &PostSettings,
    redis_bus: &mut RedisBus
    ) -> Result<(), DispatchError> {
    match message {
        Message::NewThread {
            data,
            request_id,
            remote_ip,
            board_code,
            role,
        } => {
            debug!("Thread dispatch");
            new_thread(
                db,
                listing_db,
                post_settings,
                redis_bus,
                PostBody {
                    name: data.body.name.clone(),
                    comment: data.body.comment.clone(),
                    time: data.body.time,
                    email: data.body.email.clone(),
                },
                request_id,
                remote_ip,
                board_code,
                role,
            )
            .await
        },
        Message::NewComment {
            data,
            request_id,
            remote_ip,
            board_code,
            role,
        } => {
            debug!("new comment");
            Ok(())
        },
        Message::PruneThreads {
            all_boards, board_code
        } => {
            debug!("prune threads");
            Ok(())
        }
    }
}

async fn new_thread(
    db: &Database,
    listing_db: &Database,
    post_settings: &PostSettings,
    redis_bus: &mut RedisBus,
    pb: PostBody,
    rid: &Uuid,
    rip: &IpAddr,
    board_code: &str,
    role: &Role,
) -> Result<(), DispatchError> {
    let mut errors = Vec::<PostStatus>::new();

    if pb.comment.chars().count() as i64 > post_settings.thread_comment_length {
        info!("Comment exceeded allowed length");
        errors.push(PostStatus::LargeComment);
    }

    let mut nope = false;
    if errors.is_empty() {
        let time = pb.time;
        let mut p = Thread {
            t: _thread(),
            _id: "".to_string(),
            _rev: "".to_string(),
            board_code: board_code.to_string(),
            thread_num: 20,
            body: pb,
            bump_time: time,
            archived: false,
            pinned: false,
            comments: None
        };

        let sp = p.clone();
        let mut doc = serde_json::to_value(sp).unwrap();
        match db.create(&mut doc).await {
            Ok(doc) => {
                info!("Thread (main) created");
                p._id = doc.id + "li";
                let mut listing_doc = serde_json::to_value(p).unwrap();
                match listing_db.create(&mut listing_doc).await {
                    Ok(_) => {
                        info!("Thread (listing) created");
                        redis_bus.publish("PruneThreads", "{\"board_code\": {}}").await?;
                    },
                    Err(err) => {
                        nope = true;
                        error!("error creating document {}: {:?}", listing_doc, err);
                        errors.push(PostStatus::FailedProcessing);
                    }
                }
            }
            Err(err) => {
                nope = true;
                error!("error creating document {}: {:?}", doc, err);
                errors.push(PostStatus::FailedProcessing);
            }
        };
    }

    if nope {
        return Err(DispatchError::NewThreadFailed);
    }
    
    let mut status_message_map: HashMap::<&str, &str> = HashMap::new();
    let mut expiry = 86400_i32;
    match errors.len() {
        0 => { 
            status_message_map.insert("status", "ok");
        },
        _ => {
            status_message_map.insert("status", "error");
            status_message_map.insert("errors", errors.into_iter().map(|e| e.to_string()).collect().join(", "));
            expiry = 604800;
        }
    };

    let status_json = serde_json::to_string(&status_message_map);
    match status_json {
        Ok(val) =>    match set_status(redis_bus, rid.to_string(), val, expiry).await {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("error setting post status: {:?}", e);
                return Err(DispatchError::NewThreadCreatedWithError);
            }
        },
        Err(e) => { 
                error!("error serializing post status: {:?}", e);
                return Err(DispatchError::NewThreadCreatedWithError);
        }
    }
}

async fn set_status(redis_bus: &mut RedisBus, request_id: String, message: String, duration: i32) -> Result<(), BusError> {
    match redis_bus.set_key(&request_id, message, duration).await {
        Err(e) => {
            error!("Could not set status for request id {}", request_id);
            Err(e)
        },
        Ok(_) => {
            info!("Status published for request id {}", request_id);
            Ok(())
        }
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 5)]
async fn main() -> Result<(), std::io::Error> {
    env_logger::init();
    let s = Config::builder()
        .add_source(config::File::with_name("settings"))
        .build()
        .unwrap();

    const channels: &[&str] = &["NewThread", "NewComment", "PruneThreads"];

    let client = couch_rs::Client::new(
        s.get::<String>("couch.host")
            .expect("Bad couch.host")
            .as_str(),
        s.get::<String>("couch.username")
            .expect("Bad couch.username")
            .as_str(),
        s.get::<String>("couch.password")
            .expect("Bad couch.password")
            .as_str(),
    )
    .expect("Could not create couchdb client");

    let db = client
        .db(s
            .get::<String>("couch.spriteib-db")
            .expect("Bad couch.spriteib-db")
            .as_str())
        .await
        .expect("Could not access spriteib db");

    let listing_db = client
        .db(s
            .get::<String>("couch.spriteib-listing-db")
            .expect("Bad couch.spriteib-listing-db")
            .as_str())
        .await
        .expect("Could not access spriteib listing db");

    let mut bus = RedisBus {
        uri: s
            .get::<String>("redis.connection-string")
            .expect("Bad redis.connection-string"),
        connection: None,
    };

    let post_settings = get_post_settings(s).unwrap();

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
            Ok(()) => info!("Created Redis subscription to {}", c),
            Err(e) => error!("{}", e),
        }
    }

    let db = Arc::new(db);
    let listing_db = Arc::new(listing_db);
    while let Some(msg) = ps.on_message().next().await {
        let db = db.clone();
        let listing_db = listing_db.clone();
        let mut bus = bus.clone();
        //let mut bus = bus.clone();
        tokio::task::spawn({
            async move {
                // Parse the payload
                let payload = msg.get_payload::<String>().unwrap();
                match serde_json::from_str::<Message>(&payload) {
                    Ok(m) => match dispatch_message(
                            &m,
                            &db,
                            &listing_db,
                            &post_settings,
                            &mut bus).await {
                        Ok(r) => Ok(r),
                        Err(e) => Err(e.to_string()),
                    },
                    Err(e) => {
                        warn!("Could not deserialize '{}': {}", &payload, e);
                        Err("no".to_string())
                    }
                }
            }
        });
    }

    bus.publish("x", "{\"h\": 3}").await;
    Ok(())
}
