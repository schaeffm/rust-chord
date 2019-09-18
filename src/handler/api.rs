use crate::error::MessageError;
use crate::message::api::*;
use crate::message::Message;
use crate::network::ConnectionTrait;
use crate::network::{PeerAddr, ServerHandler};
use crate::procedures::Procedures;
use crate::routing::identifier::{Identifier, Identify};
use crate::routing::Routing;
use crate::storage::Key;
use std::error::Error;
use std::io;
use std::sync::{Arc, Mutex};
use std::u8;

/// Handler for api requests
///
/// The supported incoming api messages are `DHT GET` and `DHT PUT`.
pub struct ApiHandler<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr,
{
    routing: Arc<Mutex<Routing<A>>>,
    procedures: Procedures<C, A>,
}

impl<A: PeerAddr, C: ConnectionTrait<Address = A>> ApiHandler<C, A> {
    /// Creates a new `ApiHandler` instance.
    pub fn new(routing: Arc<Mutex<Routing<A>>>, timeout: u64) -> Self {
        let procedures = Procedures::new(timeout);

        Self {
            routing,
            procedures,
        }
    }

    fn closest_preceding_peer(&self, identifier: Identifier) -> A {
        let routing = self.routing.lock().unwrap();

        **routing.closest_preceding_peer(identifier)
    }

    fn find_peer(&self, identifier: Identifier) -> crate::Result<A> {
        let closest_peer = self.closest_preceding_peer(identifier);

        self.procedures.find_peer(identifier, closest_peer)
    }

    fn handle_dht_get(&self, mut api_con: C, dht_get: DhtGet) -> crate::Result<()> {
        // iterate through all replication indices
        for i in 0..u8::MAX {
            let key = Key {
                raw_key: dht_get.key,
                replication_index: i,
            };

            let peer_addr = self.find_peer(key.identifier())?;

            if let Some(value) = self.procedures.get_value(peer_addr, key)? {
                let dht_success = DhtSuccess {
                    key: dht_get.key,
                    value,
                };
                api_con.send(Message::DhtSuccess(dht_success))?;

                return Ok(());
            }
        }

        // send failure if no value was found throughout the iteration
        let dht_failure = DhtFailure { key: dht_get.key };
        api_con.send(Message::DhtFailure(dht_failure))?;

        Ok(())
    }

    fn handle_dht_put(&self, _con: C, dht_put: DhtPut) -> crate::Result<()> {
        // iterate through all replication indices
        for i in 0..=dht_put.replication {
            let key = Key {
                raw_key: dht_put.key,
                replication_index: i,
            };

            let peer_addr = self.find_peer(key.identifier())?;

            self.procedures
                .put_value(peer_addr, key, dht_put.ttl, dht_put.value.clone())?;
        }

        Ok(())
    }

    fn handle_connection(&self, mut con: C) -> crate::Result<()> {
        let msg = con.receive()?;

        info!("Api handler received message of type {}", msg);

        match msg {
            Message::DhtGet(dht_get) => self.handle_dht_get(con, dht_get),
            Message::DhtPut(dht_put) => self.handle_dht_put(con, dht_put),
            _ => Err(Box::new(MessageError::new(msg))),
        }
    }

    fn handle_error(&self, error: &dyn Error) {
        error!("Error in ApiHandler: {}", error)
    }
}

impl<C: ConnectionTrait<Address = A>, A: PeerAddr> ServerHandler<C> for ApiHandler<C, A> {
    fn handle_connection(&self, connection: C) {
        if let Err(err) = self.handle_connection(connection) {
            self.handle_error(&*err);
        }
    }

    fn handle_error(&self, error: io::Error) {
        self.handle_error(&error)
    }
}
