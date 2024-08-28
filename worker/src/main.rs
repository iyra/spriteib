use log::{info, warn, trace, error};
use spriteib_lib::RedisBus;
use futures_util::StreamExt as _;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    env_logger::init();
    const channels: &[&str] = &["NewThread", "NewComment"];

    let mut bus = RedisBus {
        uri: "redis://localhost:6379/0".to_string(),
        connection: None
    };
    
    match bus.connect().await {
        Ok(()) => info!("Redis connection established"),
        Err(e) => panic!("{}", e)
    }

    let mut ps = match bus.pubsub().await {
        Ok(p) => {
            info!("Redis pub/sub client obtained");
            p
        },
        Err(e) => panic!("{}", e)
    };
    
    for c in channels {
        match ps.subscribe(c).await {
            Ok(()) => info!("Redis subscription to {}", c),
            Err(e) => error!("{}", e)
        }
    }

    while let Some(msg) = ps.on_message().next().await {
        tokio::task::spawn(async move {
            // Parse the payload
            let payload = msg.get_payload::<String>().unwrap();
            info!("Received message: {}", payload);
        });
    }

    bus.publish("x", "{\"h\": 3}").await;
    Ok(())
}
