//! This crate contains a [distributed hash table (DHT)][w:dht] implementation
//! based on the [Chord protocol][w:chord] using [consistent hashing][w:cons].
//!
//! # Introduction
//!
//! A DHT allows to store key-value pairs in a distributed network of peers.
//! No peer has to store the whole hash table but every node in the network
//! provides some storage and computing capacity to realize a functional
//! system.
//!
//! We distinguish between the api interface which is used to communicate with
//! other modules on the same peer and the peer-to-peer or inter-module
//! interface which allows the DHT modules on different peers to interact.
//! The DHT provides two operations via its api interface, namely PUT and GET,
//! which are used to store a value under a given key and later obtain the
//! value for this key. The peer-to-peer protocol is more complicated and
//! consists of two layers. The storage layer supports PUT and GET operations
//! just like the api interface while the routing layer implements standard
//! Chord functionality like finding a peer responsible for a given identifier.
//!
//! # Architecture Design
//!
//! ## Application Architecture
//!
//! To realize the distributed hash table we will implement the Chord protocol.
//! The central aspect of Chord is to provide a distributed lookup method. This
//! means to map a given key to a node in the network. The important aspects
//! are load balancing such that all nodes store approximately the same amount
//! of data, failure handling and recovery as described in a later section and
//! efficiency in the routing process.
//!
//! On top of this core functionality, we can implement a file storage system
//! which uses Chord to find one or several peers to store a file for a given
//! key. By separating these to layers of functionality, we can keep our
//! routing implementation as simple as possible and perform the file storage
//! operations separately. This also allows to implement redundancy and error
//! handling on a higher level.
//!
//! ## Process Architecture
//!
//! This DHT implementation is based on TCP for both the api interface and
//! the peer-to-peer communication. Therefore, we listen on two addresses given
//! in the config for the two interfaces and wait for incoming connections in
//! two event loops. Each incoming request should be handled as fast as
//! possible. Therefore we use parallelization to balance the load on several
//! cores.
//!
//! Since we need to work on shared memory between requests and also because we
//! expect each request to only take very short to process, our preferred
//! solution for parallelization is multi-threading. For this purpose, we use
//! the thread pool pattern which creates a given number of worker threads and
//! handles jobs from a job queue. Whenever a request reaches our server, it
//! creates a new task to handle this request and adds it to the queue. This
//! allows us to work concurrently while not having the overhead of spawning
//! too many threads.
//!
//! [w:dht]: https://en.wikipedia.org/wiki/Distributed_hash_table
//! [w:chord]: https://en.wikipedia.org/wiki/Chord_(peer-to-peer)
//! [w:cons]: https://en.wikipedia.org/wiki/Consistent_hashing

extern crate bigint;
extern crate byteorder;
extern crate ini;
#[macro_use]
extern crate log;
extern crate sha2;
extern crate threadpool;

use crate::config::Config;
use crate::handler::{ApiHandler, P2PHandler};
use crate::network::ConnectionTrait;
use crate::network::{PeerAddr, Server};
use crate::routing::Routing;
use crate::stabilization::{Bootstrap, Stabilization};
use std::error::Error;
use std::fmt::Display;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub mod config;
pub mod error;
pub mod handler;
pub mod key;
pub mod message;
pub mod network;
pub mod procedures;
pub mod routing;
pub mod stabilization;

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone)]
pub struct Peer<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr + Sync,
{
    config: Config<A>,
    routing: Arc<Mutex<Routing<A>>>,
    p2p_handler: Arc<P2PHandler<C, A>>,
    api_handler: Arc<ApiHandler<C, A>>,
}

impl<C, A> std::fmt::Debug for Peer<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr + Sync,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Peer {{ routing: {:?} }}", self.routing)
    }
}

impl<C, A> Peer<C, A>
where
    C: ConnectionTrait<Address = A>,
    A: PeerAddr + Sync,
{
    pub fn create(config: Config<A>) -> Result<Self> {
        let routing = {
            let finger_table = vec![config.listen_address; config.fingers];
            Routing::new(
                config.listen_address,
                config.listen_address,
                config.listen_address,
                finger_table,
                false,
            )
        };

        let routing = Arc::new(Mutex::new(routing));

        let p2p_handler = Arc::new(P2PHandler::new(Arc::clone(&routing), config.timeout));
        let api_handler = Arc::new(ApiHandler::new(Arc::clone(&routing), config.timeout));

        Ok(Peer {
            config,
            p2p_handler,
            api_handler,
            routing,
        })
    }

    pub fn run(&self, bootstrap: Option<A>, rx_peer: Receiver<()>) -> Result<()> {
        info!("Distributed Hash Table based on CHORD");
        info!("-------------------------------------\n");
        debug!(
            "The current configuration is as follows.\n\n{:#?}\n",
            self.config
        );

        let (tx_p2p, rx_p2p) = channel();
        let (tx_api, rx_api) = channel();
        let (tx_sta, rx_sta) = channel();

        let config = self.config;

        if let Some(bootstrap_address) = bootstrap {
            info!("Connection to bootstrap peer {}", bootstrap_address);

            let mut routing = self.routing.lock().unwrap();
            let bootstrap: Bootstrap<C, A> =
                Bootstrap::new(config.listen_address, bootstrap_address, config.fingers);
            *routing = bootstrap.bootstrap(config.timeout)?
        }

        let p2p_server: Server<P2PHandler<C, A>> = Server::from_arc(&self.p2p_handler);
        //let p2p_server = Server::new(Arc::clone(&self.p2p_handler));
        let p2p_handle = p2p_server.listen(config.listen_address, config.worker_threads, rx_p2p)?;

        let api_server: Server<ApiHandler<C, A>> = Server::from_arc(&self.api_handler);
        let api_handle = api_server.listen(config.api_address, 1, rx_api)?;

        let mut stabilization: Stabilization<C, A> =
            Stabilization::new(Arc::clone(&self.routing), config.timeout);
        let stabilization_handle = thread::spawn(move || loop {
            if rx_sta.try_recv().is_ok() {
                break;
            }

            if let Err(err) = stabilization.stabilize() {
                error!("Error during stabilization:\n\n{:?}", err);
            }

            thread::sleep(Duration::from_secs(config.stabilization_interval));
        });

        rx_peer.recv().unwrap();
        tx_p2p.send(()).unwrap();
        tx_api.send(()).unwrap();
        tx_sta.send(()).unwrap();

        if let Err(err) = p2p_handle.join() {
            error!("Error joining p2p handler:\n\n{:?}", err);
        }

        if let Err(err) = api_handle.join() {
            error!("Error joining api handler:\n\n{:?}", err);
        }

        if let Err(err) = stabilization_handle.join() {
            error!("Error joining stabilization:\n\n{:?}", err);
        }

        Ok(())
    }

    pub fn preds_consistent(peers: Vec<Arc<Self>>) -> bool {
        let peers: Vec<Routing<A>> = peers
            .iter()
            .map(|x| x.routing.lock().unwrap().clone())
            .collect();
        Routing::preds_consistent(peers)
    }
}

impl<C: ConnectionTrait<Address = A>, A: PeerAddr + Display + Sync> Display for Peer<C, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.p2p_handler)
    }
}
