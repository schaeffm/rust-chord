use crate::error::MessageError;
use crate::message::p2p::*;
use crate::message::Message;
use crate::network::{ConnectionTrait, PeerAddr, ServerHandler};
use crate::routing::identifier::Identifier;
use crate::routing::Routing;
use crate::storage::Key;
use std::collections::HashMap;
use std::error::Error;
use std::io;
use std::sync::{Arc, Mutex};

type Storage = HashMap<Key, Vec<u8>>;

/// Handler for peer-to-peer requests
///
/// The supported incoming peer-to-peer messages are `STORAGE GET`,
/// `STORAGE PUT`, `PEER FIND`, `PREDECESSOR GET` and `PREDECESSOR SET`.
#[derive(Debug)]
pub struct P2PHandler<A> {
    routing: Arc<Mutex<Routing<A>>>,
    storage: Mutex<Storage>,
}

impl<A: PeerAddr> P2PHandler<A> {
    /// Creates a new `P2PHandler` instance.
    pub fn new(routing: Arc<Mutex<Routing<A>>>) -> Self {
        let storage = Mutex::new(Storage::new());
        Self { routing, storage }
    }

    // FIXME? Does a node have to check / can it check if it is responsible for a key?
    fn responsible_for(&self, identifier: Identifier) -> bool {
        let routing = self.routing.lock().unwrap();

        routing.responsible_for(identifier)
    }

    fn closest_preceding_peer(&self, identifier: Identifier) -> A {
        let routing = self.routing.lock().unwrap();

        **routing.closest_preceding_peer(identifier)
    }

    fn notify_predecessor(&self, predecessor_addr: A) -> Option<A> {
        let mut routing = self.routing.lock().unwrap();

        if let Some(old_predecessor_addr) = routing.predecessor {
            // 1. check if the predecessor is closer than the previous predecessor
            if predecessor_addr.identifier().is_between(&old_predecessor_addr.identifier(), &routing.current.identifier()) {
                //if routing.responsible_for(predecessor_addr.identifier()) {
                // 2. update the predecessor if necessary
                routing.set_predecessor(predecessor_addr);

                info!("Updated predecessor to new address {}", predecessor_addr);

                // TODO maybe check whether old predecessor is actually still reachable?
                // TODO give data to new predecessor!!!
            }

            if *routing.predecessor.unwrap() == *routing.current {
                // if predecessor points to ourselves, update it to this peer.
                routing.set_predecessor(predecessor_addr);

                info!("Updated predecessor to new address {}", predecessor_addr);
            }

            if **routing.successor.first().unwrap() == *routing.current {
                // If successor points to ourselves, update it to this peer.
                routing.set_successor(predecessor_addr);

                info!("Updated successor to new address {}", predecessor_addr);
            }

            Some(*old_predecessor_addr)
        } else {
            None
        }
    }

    fn get_from_storage(&self, key: Key) -> Option<Vec<u8>> {
        let storage = self.storage.lock().unwrap();

        storage.get(&key).map(Vec::clone)
    }

    fn put_to_storage(&self, key: Key, value: Vec<u8>) -> bool {
        let mut storage = self.storage.lock().unwrap();

        if storage.contains_key(&key) {
            return false;
        }

        storage.insert(key, value);

        true
    }

    fn handle_storage_get<C: ConnectionTrait<Address=A>>(
        &self,
        mut con: C,
        storage_get: StorageGet,
    ) -> crate::Result<()> {
        let raw_key = storage_get.raw_key;
        let replication_index = storage_get.replication_index;

        let key = Key {
            raw_key,
            replication_index,
        };

        info!("Received STORAGE GET request for key {}", key);

        // 1. check if given key falls into range
        // FIXME?
        //if self.responsible_for(key.identifier()) {
        // 2. find value for given key
        let value_opt = self.get_from_storage(key);

        let msg = if let Some(value) = value_opt {
            info!(
                "Found value for key {} and replying with STORAGE GET SUCCESS",
                key
            );

            Message::StorageGetSuccess(StorageGetSuccess { raw_key, value })
        } else {
            info!(
                "Did not find value for key {} and replying with STORAGE FAILURE",
                key
            );

            Message::StorageFailure(StorageFailure { raw_key })
        };

        // 3. reply with STORAGE GET SUCCESS or STORAGE FAILURE
        con.send(msg)?;
        //}

        Ok(())
    }

    fn handle_storage_put<C: ConnectionTrait<Address=A>>(
        &self,
        mut con: C,
        storage_put: StoragePut,
    ) -> crate::Result<()> {
        let raw_key = storage_put.raw_key;
        let replication_index = storage_put.replication_index;

        let key = Key {
            raw_key,
            replication_index,
        };

        info!("Received STORAGE PUT request for key {}", key);

        // 1. check if given key falls into range
        // FIXME?
        //if self.responsible_for(key.identifier()) {
        // 2. save value for given key
        let msg = if self.put_to_storage(key, storage_put.value) {
            info!(
                "Stored value for key {} and replying with STORAGE PUT SUCCESS",
                key
            );

            Message::StoragePutSuccess(StoragePutSuccess { raw_key })
        } else {
            info!(
                "Value for key {} already exists, thus replying with STORAGE FAILURE",
                key
            );

            Message::StorageFailure(StorageFailure { raw_key })
        };

        // 3. reply with STORAGE PUT SUCCESS or STORAGE FAILURE
        con.send(msg)?;
        //}

        Ok(())
    }

    fn handle_peer_find<C>(&self, mut con: C, peer_find: PeerFind) -> crate::Result<()>
        where
            C: ConnectionTrait<Address=A>,
    {
        let current = self.routing.lock().unwrap().current.clone();
        let successor = self.routing.lock().unwrap().successor.first().unwrap().clone();
        let identifier = peer_find.identifier;

        info!("Received PEER FIND request for identifier {}", identifier);

        // check if given key falls into successor range
        if identifier.is_between_end(&current.identifier(), &successor.identifier()) {
            let socket_addr = *successor;

            info!("Replying with PEER FOUND with address {}", socket_addr);

            let peer_found = PeerFound {
                identifier,
                socket_addr,
            };
            con.send(Message::PeerFound(peer_found))?;
        } else {
            // get closest preceding peer to send a PeerFind request
            let peer_addr = self.closest_preceding_peer(identifier);
            //println!("next for {:?}: {:?}", identifier, peer_addr);
            let mut new_con = C::open(peer_addr, 3600)?;
            let peer_find = PeerFind { identifier };
            new_con.send(Message::PeerFind(peer_find))?;
            let msg = new_con.receive()?;

            if let Message::PeerFound(peer_found) = msg {
                let socket_addr = peer_found.socket_addr;

                // reply to original request with PEER FOUND
                info!("Replying with PEER FOUND with address {}", socket_addr);

                let peer_found = PeerFound {
                    identifier,
                    socket_addr,
                };

                con.send(Message::PeerFound(peer_found))?;
            } else {
                return Err(Box::new(MessageError::new(msg)));
            }
        }

        Ok(())
    }

    fn handle_successors_request<C>(
        &self,
        mut con: C) -> crate::Result<()>
        where
            C: ConnectionTrait<Address=A>, {
        let routing = self.routing.lock().unwrap();
        let mut successors = routing.successor.iter().map(|x| **x).collect();
        let mut successors_pred = match routing.predecessor {
            None => vec![],
            Some(p) => vec![*p],
        };
        successors_pred.push(*routing.current);
        successors_pred.append(&mut successors);
        let successor_reply = SuccessorsReply{
            successors: successors_pred };
        con.send(Message::SuccessorsReply(successor_reply))?;

        Ok(())
    }

    fn handle_predecessor_notify<C>(
        &self,
        mut con: C,
        predecessor_notify: PredecessorNotify<A>,
    ) -> crate::Result<()>
        where
            C: ConnectionTrait<Address=A>,
    {
        let predecessor_addr = predecessor_notify.socket_addr;

        info!("Received PREDECESSOR GET request from {}", predecessor_addr);

        if let Some(socket_addr) = self.notify_predecessor(predecessor_addr) {
            info!(
                "Replying with PREDECESSOR FOUND and address {}",
                socket_addr
            );

            // 3. return the current predecessor with PREDECESSOR FOUND
            let predecessor_found = PredecessorFound { socket_addr };
            con.send(Message::PredecessorFound(predecessor_found))?;
        } else {
            info!(
                "Replying with PREDECESSOR NOT FOUND"
            );

            // 3. return the current predecessor with PREDECESSOR FOUND
            con.send(Message::PredecessorNotFound)?;
        }

        Ok(())
    }

    fn handle_connection<C>(&self, mut con: C) -> crate::Result<()>
        where
            C: ConnectionTrait<Address=A>,
    {
        let msg = con.receive()?;

        info!("P2P handler received message of type {}", msg);

        match msg {
            Message::StorageGet(storage_get) => self.handle_storage_get(con, storage_get),
            Message::StoragePut(storage_put) => self.handle_storage_put(con, storage_put),
            Message::PeerFind(peer_find) => self.handle_peer_find(con, peer_find),
            Message::SuccessorsRequest() => self.handle_successors_request(con),
            Message::PredecessorNotify(predecessor_get) => {
                self.handle_predecessor_notify(con, predecessor_get)
            }
            _ => Err(Box::new(MessageError::new(msg))),
        }
    }

    fn handle_error(&self, error: &dyn Error) {
        error!("Error in P2PHandler: {}", error)
    }
}

impl<A, C> ServerHandler<C> for P2PHandler<A>
    where
        C: ConnectionTrait<Address=A>,
        A: PeerAddr,
{
    fn handle_connection(&self, connection: C) {
        if let Err(err) = P2PHandler::handle_connection(&self, connection) {
            self.handle_error(&*err);
        }
    }

    fn handle_error(&self, error: io::Error) {
        self.handle_error(&error)
    }
}
