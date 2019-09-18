//! A collection of procedures used in various places.

use crate::error::MessageError;
use crate::message::p2p::{PeerFind, PredecessorNotify, StorageGet, StoragePut};
use crate::message::Message;
use crate::network::ConnectionTrait;
use crate::network::PeerAddr;
use crate::routing::identifier::Identifier;
use crate::storage::Key;
use std::marker::PhantomData;
use std::sync::Mutex;

pub struct Procedures<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr,
{
    timeout: u64,
    p: PhantomData<Mutex<C>>,
}

impl<C: ConnectionTrait<Address = A>, A: PeerAddr> Procedures<C, A> {
    pub fn new(timeout: u64) -> Self {
        Self {
            timeout,
            p: PhantomData,
        }
    }

    /// Get the socket address of the peer responsible for a given identifier.
    ///
    /// This iteratively sends PEER FIND messages to successive peers,
    /// beginning with `peer_addr` which could be taken from a finger table.
    pub fn find_peer(&self, identifier: Identifier, peer_addr: A) -> crate::Result<A> {
        debug!("Finding peer for identifier {}", identifier);

        // TODO use successors from list if PeerNotFound
        //loop {
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

        /*
            if reply_addr == peer_addr {
                debug!(
                    "Peer found for identifier {} with address {}",
                    identifier, reply_addr
                );

                return Ok(reply_addr);
            }
        */

            //peer_addr = reply_addr;
        //}
    }

    /// Send a storage get message to a peer with the objective to find a value for a given key.
    ///
    /// Opens a P2P connection to `peer_addr` and sends a STORAGE GET message to retrieve a value for
    /// `key` depending on the reply.
    pub fn get_value(&self, peer_addr: A, key: Key) -> crate::Result<Option<Vec<u8>>> {
        debug!("Get value for key {} from peer {}", key, peer_addr);

        let storage_get = StorageGet {
            replication_index: key.replication_index,
            raw_key: key.raw_key,
        };

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
    pub fn put_value(&self, peer_addr: A, key: Key, ttl: u16, value: Vec<u8>) -> crate::Result<()> {
        debug!("Put value for key {} to peer {}", key, peer_addr);

        let storage_put = StoragePut {
            ttl,
            replication_index: key.replication_index,
            raw_key: key.raw_key,
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
    /// reply with the socket addres of `socket_addr`.
    pub fn notify_predecessor(&self, socket_addr: A, peer_addr: A) -> crate::Result<A> {
        debug!("Getting predecessor of peer {}", peer_addr);

        let mut con = C::open(peer_addr, self.timeout)?;

        con.send(Message::PredecessorNotify(PredecessorNotify {
            socket_addr,
        }))?;

        let msg = con.receive()?;

        if let Message::PredecessorFound(predecessor_reply) = msg {
            info!("Predecessor received from peer {}", peer_addr);

            Ok(predecessor_reply.socket_addr)
        } else {
            warn!("No predecessor received from peer {}", peer_addr);

            Err(Box::new(MessageError::new(msg)))
        }
    }
}
