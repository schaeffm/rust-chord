use crate::error::MessageError;
use crate::message::p2p::*;
use crate::message::Message;
use crate::network::{ConnectionTrait, PeerAddr, ServerHandler};
use crate::procedures::Procedures;
use crate::routing::identifier::Identifier;
use crate::routing::Routing;
use std::collections::HashMap;
use std::collections::HashSet;
use std::error::Error;
use std::fmt::Display;
use std::io;
use std::iter::FromIterator;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

type Storage = HashMap<Identifier, Vec<u8>>;

/// Handler for peer-to-peer requests
///
/// The supported incoming peer-to-peer messages are `STORAGE GET`,
/// `STORAGE PUT`, `PEER FIND`, `PREDECESSOR GET` and `PREDECESSOR SET`.
#[derive(Debug)]
pub struct P2PHandler<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr,
{
    routing: Arc<Mutex<Routing<A>>>,
    storage: Arc<Mutex<Storage>>,
    procedures: Procedures<C, A>,
    p: PhantomData<Mutex<C>>,
}

impl<C, A> P2PHandler<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr,
{
    /// Creates a new `P2PHandler` instance.
    pub fn new(routing: Arc<Mutex<Routing<A>>>, timeout: u64) -> Self {
        let procedures = Procedures::new(timeout);
        let storage = Arc::new(Mutex::new(Storage::new()));
        Self {
            routing,
            storage,
            procedures,
            p: PhantomData,
        }
    }

    fn closest_preceding_peer(&self, identifier: Identifier) -> A {
        let routing = self.routing.lock().unwrap();

        **routing.closest_preceding_peer(identifier)
    }

    fn key_range(&self, start: &Identifier, end: &Identifier) -> Vec<Identifier> {
        self.storage
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .filter(|k| k.is_between_end(start, end))
            .collect()
    }

    fn notify_predecessor(&self, new_predecessor: A) {
        let (predecessor, current) = {
            let routing = self.routing.lock().unwrap();
            (routing.predecessor, routing.current)
        };

        let failed = self.routing.lock().unwrap().get_predecessor_failed();

        if failed && predecessor.identifier() == current.identifier() {
            // only occurs right after joining the network
            self.routing
                .lock()
                .unwrap()
                .set_predecessor(new_predecessor);
        } else if failed
            && predecessor
                .identifier()
                .is_between(&new_predecessor.identifier(), &current.identifier())
        {
            // old predecessor failed, we get a new predecessor:
            // keys in range (new_predecessor, predecessor) are sent to all successors

            let new_pred_keys =
                self.key_range(&new_predecessor.identifier(), &predecessor.identifier());

            for successor in self.routing.lock().unwrap().successor.clone() {
                self.put_keys(*successor, &new_pred_keys);
            }

            self.routing
                .lock()
                .unwrap()
                .set_predecessor(new_predecessor);
        } else {
            let predecessor = self.routing.lock().unwrap().predecessor;
            if new_predecessor
                .identifier()
                .is_between(&predecessor.identifier(), &current.identifier())
            {
                // node join -> set new predecessor
                self.routing
                    .lock()
                    .unwrap()
                    .set_predecessor(new_predecessor);
                let new_pred_keys =
                    self.key_range(&predecessor.identifier(), &new_predecessor.identifier());
                self.put_keys(new_predecessor, &new_pred_keys);

                // The last item in the successor is no longer required to store keys transfered to
                // the new predecessor
                if let Some(last_succ) = self.routing.lock().unwrap().last_successor() {
                    self.remove_keys(**last_succ, &new_pred_keys);
                }
            }
        }
    }

    fn get_from_storage(&self, key: Identifier) -> Option<Vec<u8>> {
        let storage = self.storage.lock().unwrap();

        storage.get(&key).map(Vec::clone)
    }

    fn put_to_storage(&self, key: Identifier, value: Vec<u8>) -> bool {
        let mut storage = self.storage.lock().unwrap();

        if storage.contains_key(&key) {
            return false;
        }

        storage.insert(key, value);

        true
    }

    fn handle_storage_get(&self, mut con: C, storage_get: StorageGet) -> crate::Result<()> {
        let key = storage_get.key;

        info!("Received STORAGE GET request for key {}", key);

        // 1. find value for given key (even if not responsible)
        let value_opt = self.get_from_storage(key);

        let msg = if let Some(value) = value_opt {
            info!(
                "Found value for key {} and replying with STORAGE GET SUCCESS",
                key
            );

            Message::StorageGetSuccess(StorageGetSuccess { key, value })
        } else {
            info!(
                "Did not find value for key {} and replying with STORAGE FAILURE",
                key
            );

            Message::StorageFailure(StorageFailure { key })
        };

        // 2. reply with STORAGE GET SUCCESS or STORAGE FAILURE
        con.send(msg)?;

        Ok(())
    }

    fn handle_storage_put(&self, mut con: C, storage_put: StoragePut) -> crate::Result<()> {
        let key = storage_put.key;

        info!("Received STORAGE PUT request for key {}", key);

        // 1. save value for given key (even if not responsible)
        let msg = if self.put_to_storage(key, storage_put.value.clone()) {
            info!(
                "Stored value for key {} and replying with STORAGE PUT SUCCESS",
                key
            );

            Message::StoragePutSuccess(StoragePutSuccess { key })
        } else {
            info!(
                "Value for key {} already exists, thus replying with STORAGE FAILURE",
                key
            );

            Message::StorageFailure(StorageFailure { key })
        };

        // 2. reply with STORAGE PUT SUCCESS or STORAGE FAILURE
        con.send(msg)?;

        let successors = self.routing.lock().unwrap().successor.clone();
        if self.routing.lock().unwrap().responsible_for(key) {
            for succ in successors {
                self.procedures.put_value(
                    *succ,
                    key,
                    storage_put.ttl,
                    storage_put.value.clone(),
                )?;
            }
        }

        Ok(())
    }

    fn handle_peer_find(&self, mut con: C, peer_find: PeerFind) -> crate::Result<()> {
        let (current, successor) = {
            let routing = self.routing.lock().unwrap();
            (
                *routing.current,
                *routing.successor.first().ok_or("Empty successor list")?,
            )
        };
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

    fn handle_successors_request(&self, mut con: C) -> crate::Result<()> {
        let routing = self.routing.lock().unwrap();
        let mut successors = routing.successor.iter().map(|x| **x).collect();
        let mut successors_pred = if routing.get_predecessor_failed() {
            vec![]
        } else {
            vec![*routing.predecessor]
        };

        successors_pred.push(*routing.current);
        successors_pred.append(&mut successors);
        let successor_reply = SuccessorsReply {
            successors: successors_pred,
        };
        con.send(Message::SuccessorsReply(successor_reply))?;

        Ok(())
    }

    fn handle_predecessor_notify(
        &self,
        predecessor_notify: PredecessorNotify<A>,
    ) -> crate::Result<()> {
        let predecessor_addr = predecessor_notify.socket_addr;
        self.notify_predecessor(predecessor_addr);
        Ok(())
    }

    fn put_keys(&self, peer_addr: A, keys: &[Identifier]) {
        if !keys.is_empty() {
            let connect = C::open(peer_addr, 3600);
            let msg = Message::KeyPut(KeyPut {
                keys: keys.to_vec(),
            });
            if connect.and_then(|mut con| con.send(msg)).is_err() {
                info!(
                    "Failed to notify {} about keys it is responsible for.",
                    peer_addr
                );
            }
        }
    }

    fn remove_keys(&self, peer_addr: A, keys: &[Identifier]) {
        if !keys.is_empty() {
            let connect = C::open(peer_addr, 3600);
            let msg = Message::KeyRemove(KeyRemove {
                keys: keys.to_vec(),
            });
            if connect.and_then(|mut con| con.send(msg)).is_err() {
                info!(
                    "Failed to notify {} about keys it is no longer responsible for.",
                    peer_addr
                );
            }
        }
    }

    fn handle_successor_changes(
        &self,
        successor_changes: SuccessorListChanges<A>,
    ) -> crate::Result<()> {
        let old_successors_set: HashSet<_> =
            HashSet::from_iter(successor_changes.old_successors.iter().cloned());
        let new_successors_set: HashSet<_> =
            HashSet::from_iter(successor_changes.new_successors.iter().cloned());

        let current = self.routing.lock().unwrap().current;

        let own_keys: Vec<Identifier> = self
            .storage
            .lock()
            .unwrap()
            .keys()
            .filter(|k| self.routing.lock().unwrap().responsible_for(**k))
            .cloned()
            .collect();

        // all peers that are not in the successors list anymore
        for peer in old_successors_set.difference(&new_successors_set) {
            if peer.identifier() != current.identifier() {
                println!(
                    "{} sends a remove key message (succs changed) to {:?} containing {:?}",
                    *current, *peer, &own_keys
                );
                self.remove_keys(*peer, &own_keys);
            }
        }

        // all peers that are new in the successors list
        for peer in new_successors_set.difference(&old_successors_set) {
            if peer.identifier() != current.identifier() {
                println!(
                    "{} sends a put key message (succs changed) to {:?} containing {:?}",
                    *current, *peer, &own_keys
                );
                self.put_keys(*peer, &own_keys);
            }
        }

        Ok(())
    }

    fn handle_connection(&self, mut con: C) -> crate::Result<()> {
        let msg = con.receive()?;

        info!("P2P handler received message of type {}", msg);

        match msg {
            Message::StorageGet(storage_get) => self.handle_storage_get(con, storage_get),
            Message::StoragePut(storage_put) => self.handle_storage_put(con, storage_put),
            Message::PeerFind(peer_find) => self.handle_peer_find(con, peer_find),
            Message::SuccessorsRequest(_) => self.handle_successors_request(con),
            Message::PredecessorNotify(predecessor_get) => {
                self.handle_predecessor_notify(predecessor_get)
            }
            Message::SuccessorlistChanges(successor_changes) => {
                self.handle_successor_changes(successor_changes)
            }
            Message::KeyPut(put_keys) => self.handle_put_keys(put_keys),
            Message::KeyRemove(remove_keys) => self.handle_remove_keys(remove_keys),
            _ => Err(Box::new(MessageError::new(msg))),
        }
    }

    fn handle_remove_keys(&self, remove_keys: KeyRemove) -> crate::Result<()> {
        let mut storage = self.storage.lock().unwrap();

        println!(
            "{} is going to remove {:?}",
            self.routing.lock().unwrap().current.identifier(),
            remove_keys
        );

        for key in remove_keys.keys {
            storage.remove(&key);
        }

        Ok(())
    }

    fn handle_put_keys(&self, put_keys: KeyPut) -> crate::Result<()> {
        // TODO: implement concurrency
        for key in put_keys.keys {
            if !self.storage.lock().unwrap().contains_key(&key) {
                let next = *self.routing.lock().unwrap().closest_preceding_peer(key);
                if let Ok(Some(value)) = self.procedures.dht_get(key, *next) {
                    self.put_to_storage(key, value);
                }
            }
        }

        Ok(())
    }

    fn handle_error(&self, error: &dyn Error) {
        error!("Error in P2PHandler: {}", error)
    }
}

impl<A, C> ServerHandler<C> for P2PHandler<C, A>
where
    C: ConnectionTrait<Address = A>,
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

impl<C: ConnectionTrait<Address = A>, A: PeerAddr + Display + Sync> Display for P2PHandler<C, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let routing = self.routing.try_lock();
        let storage = self.storage.try_lock();

        match (routing, storage) {
            (Ok(routing), Ok(storage)) => {
                let pred = *routing.predecessor;

                let successors: Vec<_> = routing.successor.iter().map(|a| **a).collect();
                let fingers: Vec<_> = routing.finger_table.iter().map(|a| **a).collect();
                write!(
                    f,
                    "Predecessor: {}\n\
                     Successors: {:?}\n\
                     Fingers: {:?}\n\
                     Storage: {}\n",
                    pred,
                    successors,
                    fingers,
                    display_storage(&storage)
                )
            }
            _ => write!(f, "Peer locked... (probably failed)"),
        }
    }
}

fn display_storage(storage: &Storage) -> String {
    let mut tmp = String::new();

    for (k, v) in storage.iter() {
        let hex = k.as_bytes()[..8]
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(":");

        tmp.push_str(&format!(
            "[{}..: {}], ",
            hex,
            std::str::from_utf8(v).unwrap()
        ));
    }

    tmp
}
