//! A collection of procedures used in various places.

use crate::error::MessageError;
use crate::message::p2p::{PeerFind, PredecessorNotify, StorageGet, StoragePut, SuccessorListChanges, SuccessorsRequest};
use crate::message::Message;
use crate::network::ConnectionTrait;
use crate::network::PeerAddr;
use crate::routing::identifier::{Identifier, IdentifierValue};
use std::marker::PhantomData;
use std::sync::Mutex;

#[derive(Debug)]
pub struct Procedures<C, A>
    where
        C: ConnectionTrait<Address=A>,
        A: PeerAddr,
{
    timeout: u64,
    p: PhantomData<Mutex<C>>,
}

impl<C: ConnectionTrait<Address=A>, A: PeerAddr> Procedures<C, A> {
    pub fn new(timeout: u64) -> Self {
        Self {
            timeout,
            p: PhantomData,
        }
    }

    /// Get the socket address of the peer responsible for a given identifier.
    ///
    /// The lookup is implemented recursively via PEER FIND messages,
    /// beginning with `peer_addr` which could be taken from a finger table.
    pub fn find_peer(&self, identifier: Identifier, peer_addr: A) -> crate::Result<A> {
        debug!("Finding peer for identifier {}", identifier);
        // TODO use successors from list if PeerNotFound
        let mut con = C::open(peer_addr, self.timeout)?;
        let peer_find = PeerFind { identifier };
        con.send(Message::PeerFind(peer_find))?;
        let msg = con.receive()?;

        if let Message::PeerFound(peer_found) = msg {
            debug!(
                "Peer found for identifier {} with address {}",
                identifier, peer_found.socket_addr
            );

            Ok(peer_found.socket_addr)
        } else {
            Err(Box::new(MessageError::new(msg)))
        }
    }

    /// Send a storage get message to a peer with the objective to find a value for a given key.
    ///
    /// Opens a P2P connection to `peer_addr` and sends a STORAGE GET message to retrieve a value for
    /// `key` depending on the reply.
    pub fn get_value(&self, peer_addr: A, key: Identifier) -> crate::Result<Option<Vec<u8>>> {
        debug!("Get value for key {} from peer {}", key, peer_addr);

        let storage_get = StorageGet { key };

        let mut p2p_con = C::open(peer_addr, 3600)?;
        p2p_con.send(Message::StorageGet(storage_get))?;

        let msg = p2p_con.receive()?;

        if let Message::StorageGetSuccess(storage_success) = msg {
            info!(
                "Value for key {} successfully received from peer {}",
                key, peer_addr
            );

            Ok(Some(storage_success.value))
        } else {
            warn!("No value found for key {} at peer {}", key, peer_addr);

            Ok(None)
        }
    }

    /// Put a value for a given key into the distributed hash table.
    ///
    /// Opens a P2P connection to `peer_addr` and sends a STORAGE PUT message to store `value` under `key`.
    pub fn put_value(&self, peer_addr: A, key: Identifier, ttl: u16, value: Vec<u8>) -> crate::Result<()> {
        debug!("Put value for key {} to peer {}", key, peer_addr);

        let storage_put = StoragePut {
            ttl,
            key,
            value,
        };

        let mut p2p_con = C::open(peer_addr, 3600)?;
        p2p_con.send(Message::StoragePut(storage_put))?;

        let msg = p2p_con.receive()?;

        if let Message::StoragePutSuccess(_) = msg {
            info!(
                "Value for key {} succe
        let peers: Vec<&Routing<A>> = peers.iter().collect();ssfully stored at peer {}",
                key, peer_addr
            );

            return Ok(());
        }

        if let Message::StorageFailure(_) = msg {
            warn!(
                "Key {} exists already in storage of peer {}",
                key, peer_addr
            );

            return Ok(());
        }

        Err(Box::new(MessageError::new(msg)))
    }

    /// Notify the successor of a potential predecessor and asks to reply with the current predecessor.
    ///
    /// Opens a P2P connection and sends a PREDECESSOR NOTIFY message to `peer_addr` to receive a
    /// reply with the socket address of `socket_addr`.
    pub fn notify(&self, socket_addr: A, peer_addr: A) {
        let connect = C::open(peer_addr, self.timeout);
        let reply = Message::PredecessorNotify(PredecessorNotify { socket_addr });
        if let Err(_) = connect.and_then(|mut con| con.send(reply)) {
            info!("Failed to notify {}", peer_addr);
        }
    }

    pub fn get_successors(&self, socket_addr: A, peer_addr: A) -> crate::Result<Vec<IdentifierValue<A>>> {
        debug!("Getting successors of peer {}", peer_addr);

        let mut con = C::open(peer_addr, self.timeout)?;

        con.send(Message::SuccessorsRequest(SuccessorsRequest {}))?;

        let msg = con.receive()?;

        if let Message::SuccessorsReply(successors_reply) = msg {
            info!("Successors received from peer {}", peer_addr);
            let mut successors = successors_reply.successors.into_iter().peekable();
            if let Some(fst) = &successors.peek() {
                if !fst.identifier().
                    is_between_end(&socket_addr.identifier(), &peer_addr.identifier()) {
                    successors.next();
                }
            }

            Ok(successors.take(4).map(IdentifierValue::new).collect())
        } else {
            warn!("No successors received from peer {}", peer_addr);

            Err(Box::new(MessageError::new(msg)))
        }
    }

    pub fn send_successor_changes(&self, socket_addr: A, old_successors: Vec<A>, new_successors: Vec<A>) {
        let connect = C::open(socket_addr, self.timeout);
        let reply = Message::SuccessorlistChanges(SuccessorListChanges { old_successors, new_successors });
        if let Err(_) = connect.and_then(|mut con| con.send(reply)) {
            info!("Failed to update own successors {}", socket_addr);
        }
    }

    pub fn dht_get(&self, key: Identifier, addr: A) -> crate::Result<Option<Vec<u8>>> {
        let peer_addr = self.find_peer(key, addr)?;
        self.get_value(peer_addr, key)
    }
}
