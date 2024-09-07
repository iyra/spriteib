use config::Config;
use couch_rs::database::Database;
use futures_util::StreamExt as _;
use log::{debug, error, info, warn};
use spriteib_lib::{
    DispatchError, Message, PostBody, RedisBus, Role, Thread, _thread, PostStatus, get_sprite_settings, get_couch_settings,
    get_redis_settings, SpriteSettings
};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;
use std::collections::HashMap;

async fn dispatch_message(
    message: &Message,
    db: &Database,
    listing_db: &Database,
    post_settings: &SpriteSettings,
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
                data.body.clone(),
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
        },
        Message::PublishRss {
            all_boards, board_code
        } => {
            debug!("publish rss");
            Ok(())
        }
    }
}

async fn new_thread(
    db: &Database,
    listing_db: &Database,
    post_settings: &SpriteSettings,
    redis_bus: &mut RedisBus,
    pb: PostBody,
    rid: &Uuid,
    rip: &IpAddr,
    board_code: &str,
    role: &Role,
) -> Result<(), DispatchError> {
    let mut errors = Vec::<PostStatus>::new();

    if pb.comment.chars().count() as i64 > post_settings.post_op_max_length {
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

        let mut doc = serde_json::to_value(&p).unwrap();
        match db.create(&mut doc).await {
            Ok(doc) => {
                info!("Thread (main) created");
                p._id = doc.id + "li";
                let mut listing_doc = serde_json::to_value(&p).unwrap();
                match listing_db.create(&mut listing_doc).await {
                    Ok(_) => {
                        info!("Thread (listing) created");
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
    
    let mut status_message_map: HashMap::<&str, String> = HashMap::new();
    let mut expiry = 86400_i32;
    match errors.len() {
        0 => { 
            status_message_map.insert("status", "ok".to_string());
        },
        _ => {
            status_message_map.insert("status", "error".to_string());
            let err_str = errors
                    .into_iter().map(|e| e.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
                    .to_string();

            status_message_map.insert("errors", err_str);
            expiry = 604800;
        }
    };

    let status_json = serde_json::to_string(&status_message_map);
    match status_json {
        Ok(val) =>    match redis_bus.set_status(rid.to_string(), val, expiry).await {
            Ok(_) => {
                let prune_msg = serde_json::to_string(&Message::PruneThreads { all_boards: false, board_code: Some(board_code.to_string()) }).map_err(|e| DispatchError::NewThreadCreatedWithError)?;
                let publish_rss = serde_json::to_string(&Message::PublishRss { all_boards: false, board_code: Some(board_code.to_string()) }).map_err(|x| DispatchError::NewThreadCreatedWithError)?;

                match redis_bus.publish("PruneThreads", &prune_msg).await.and(
                        redis_bus.publish("PublishRss", &publish_rss).await) {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            error!("failed to send refresh messages after creathon: {:?}", e);
                            Err(DispatchError::NewThreadCreatedWithError)
                        }
                    }
            },
            Err(e) => {
                error!("error setting post status: {:?}", e);
                Err(DispatchError::NewThreadCreatedWithError)
            }
        },
        Err(e) => { 
                error!("error serializing post status: {:?}", e);
                Err(DispatchError::NewThreadCreatedWithError)
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

    const channels: &[&str] = &["NewThread", "NewComment", "PruneThreads", "PublishRss"];

    let sprite_settings = get_sprite_settings(&s).unwrap();
    let couch_settings = get_couch_settings(&s).unwrap();
    let redis_settings = get_redis_settings(&s).unwrap();

    let client = couch_rs::Client::new(
        &couch_settings.host,
        &couch_settings.username,
        &couch_settings.password
    ).expect("Could not create couchdb client");

    let db = client
        .db(&couch_settings.db_listing)
        .await
        .expect("Could not access spriteib db");

    let listing_db = client
        .db(&couch_settings.db_spriteib)
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
        let sprite_settings = sprite_settings.clone();

        tokio::task::spawn({
            async move {
                // Parse the payload
                let payload = msg.get_payload::<String>().unwrap();
                match serde_json::from_str::<Message>(&payload) {
                    Ok(m) => match dispatch_message(
                            &m,
                            &db,
                            &listing_db,
                            &sprite_settings,
                            &mut bus).await {
                        Ok(r) => Ok(r),
                        Err(e) => {
                            error!("Dispatch failed, {:?}", e);
                            Err(e.to_string())
                        },
                    },
                    Err(e) => {
                        warn!("Could not deserialize '{}': {}", &payload, e);
                        Err("no".to_string())
                    }
                }
            }
        });
    }

    bus.publish("x", "{\"h\": 3}").await.unwrap();
    Ok(())
}
