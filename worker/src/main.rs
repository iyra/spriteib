use redis::Connection;
use redis::Cmd;
use redis::aio::{MultiplexedConnection, PubSub};
use redis::{AsyncCommands, Client, ConnectionInfo, RedisError};
use std::error;
use serde::ser::{Serialize};
use serde_json::{json, Value};
use futures_util::StreamExt as _;
use log::{info, warn, trace, error};

pub struct RedisBus {
    pub uri: String,
    connection: Option<MultiplexedConnection>,
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
