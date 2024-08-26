use redis::Connection;
use redis::Cmd;
use redis::aio::{MultiplexedConnection, PubSub};
use redis::{AsyncCommands, Client, ConnectionInfo, RedisError};
use std::error;
use serde::ser::{Serialize};

pub struct RedisBus {
    pub uri: String,
}

pub fn publish_message(conn: &mut Connection, channel: &str, message: &str) -> Result<(), Box<dyn std::error::Error>>
{
    //let json = serde_json::to_string(message)?;
    let p = Cmd::publish(channel, message);
    p.exec(conn)?;
    Ok(())
}

impl RedisBus {
    pub async fn connect(&mut self) -> Result<MultiplexedConnection, RedisError> {
        let client = redis::Client::open(self.uri.clone()).unwrap();
        let connection = client.get_multiplexed_async_connection().await?;
        Ok(connection)
    }

    pub async fn pubsub(&mut self) -> Result<PubSub, RedisError> {
        // since pubsub performs a multicast for all nodes in a cluster,
        // listening to a single server in the cluster is sufficient for cluster setups
        let client = Client::open(self.uri.clone())?;
        client.get_async_pubsub().await
    }

    pub async fn publish(&mut self, channel: &str, message: &str) {
        let ps = &mut self.connect().await;
        match ps {
            Ok(mp) => {
                match mp.publish(channel, message.to_string()).await {
                    Ok(data) => data,
                    Err(e) => {
                    } 
                }
            }
            Err(e) => println!("z")
        }
    }
}
