#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate stderrlog;

use dht::config::Config;
use dht::message::api::{DhtGet, DhtPut};
use dht::message::Message;
use dht::network::ConnectionTrait;
use dht::routing::identifier::Identifier;
use dht::routing::identifier::Identify;
use dht::{Peer, Result};
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::process;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

type MockSyn = (Receiver<MockMsg>, Sender<Receiver<MockMsg>>);
type ConnChannel = Sender<MockSyn>;
type MockMsg = Message<MockAddr>;
type MockListener = Receiver<MockSyn>;

struct MockNetwork {
    peers: HashMap<MockAddr, ConnChannel>,
}

impl MockNetwork {
    fn new() -> Self {
        MockNetwork {
            peers: HashMap::new(),
        }
    }
}

lazy_static! {
    static ref NETWORK: Mutex<MockNetwork> = Mutex::new(MockNetwork::new());
    static ref COMMANDS: HashMap<&'static str, &'static str> = [
        ("put", "Store key-value pair in DHT"),
        ("get", "Retrieve value for some key from DHT"),
        ("help", "Display this message"),
        ("inspect", "Inspect the network"),
        ("quit", "Exit the program"),
        ("join", "Add a new node to the network"),
        ("create", "Open a new network"),
        ("leave", "Remove a node from the network"),
        ("list", "List all nodes in the network"),
        ("consistency", "Check the network for consistency"),
    ]
    .iter()
    .cloned()
    .collect();
}

struct MockConn {
    sender: Sender<MockMsg>,
    receiver: Receiver<MockMsg>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum MockAddr {
    ApiAddr(u64),
    P2PAddr(u64),
}

impl std::fmt::Debug for MockAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            MockAddr::P2PAddr(addr) => write!(f, "{}", addr),
            MockAddr::ApiAddr(addr) => write!(f, "ApiAddr {}", addr),
        }
    }
}

impl std::fmt::Display for MockAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl Identify for MockAddr {
    fn identifier(&self) -> Identifier {
        let addr = match self {
            MockAddr::ApiAddr(addr) | MockAddr::P2PAddr(addr) => addr,
        };
        Identifier::new(&addr.to_be_bytes())
    }
}

impl ConnectionTrait for MockConn {
    type Address = MockAddr;
    type Listener = MockListener;

    fn open(peer_addr: Self::Address, timeout_ms: u64) -> Result<Self> {
        let network = NETWORK.lock()?;
        let peer_ch = network.peers.get(&peer_addr).ok_or("Connection failed")?;
        let (sender, receiver_peer) = channel();
        let (sender_conn, receiver_conn) = channel();
        peer_ch.send((receiver_peer, sender_conn))?;
        drop(network);
        let receiver = receiver_conn.recv_timeout(Duration::from_millis(timeout_ms))?;
        Ok(MockConn { receiver, sender })
    }

    fn receive(&mut self) -> Result<Message<Self::Address>> {
        Ok(self.receiver.recv()?)
    }

    fn send(&mut self, msg: Message<Self::Address>) -> Result<()> {
        Ok(self.sender.send(msg)?)
    }

    fn shutdown(self) -> Result<()> {
        drop(self.receiver);
        drop(self.sender);
        Ok(())
    }

    fn bind(addr: Self::Address) -> Result<Self::Listener> {
        let (sender, receiver) = channel();
        NETWORK.lock()?.peers.insert(addr, sender);
        Ok(receiver)
    }

    fn listen<'a>(listener: &'a Self::Listener) -> Box<dyn 'a + Iterator<Item = Self>> {
        Box::new(listener.iter().map(move |(receiver, sender_ch)| {
            let (sender, receiver_peer) = channel();
            sender_ch.send(receiver_peer).ok();
            MockConn { sender, receiver }
        }))
    }
}

fn join(addr: u64, bootstrap: u64) -> Arc<Peer<MockConn, MockAddr>> {
    create_peer(addr, Some(MockAddr::P2PAddr(bootstrap)))
}

fn create(addr: u64) -> Arc<Peer<MockConn, MockAddr>> {
    create_peer(addr, None)
}

fn create_peer(addr: u64, bootstrap: Option<MockAddr>) -> Arc<Peer<MockConn, MockAddr>> {
    let config = Config {
        listen_address: MockAddr::P2PAddr(addr),
        api_address: MockAddr::ApiAddr(addr),
        worker_threads: 5,
        timeout: 1000,
        fingers: 4,
        stabilization_interval: 30,
    };

    let peer = Arc::new(Peer::<MockConn, MockAddr>::create(config).unwrap());
    {
        let peer = peer.clone();
        thread::spawn(move || {
            if let Err(e) = peer.run(bootstrap) {
                error!("Application error: {}", e);
                process::exit(1);
            }
        });
    }
    peer
}

fn main() {
    stderrlog::new().module(module_path!()).init().unwrap();

    let mut peers = HashMap::new();
    peers.insert(1, create(1));
    peers.insert(2, join(2, 1));

    println!("Client to talk to the DHT Mock App");
    println!("-----------------------------\n");

    handle_help();

    loop {
        let command = read_line("Enter a command").unwrap();

        match command.as_ref() {
            "put" => handle_put(),
            "get" => handle_get(),
            "inspect" => handle_inspect(&peers),
            "join" => handle_join(&mut peers),
            _ => handle_help(),
        }
    }
}

fn read_line(question: &str) -> Option<String> {
    print!("{}: ", question);
    io::stdout().flush().unwrap();

    let mut line = String::new();

    match io::stdin().read_line(&mut line) {
        Ok(_) => Some(line.trim().to_string()),
        Err(err) => {
            eprintln!("Error: {}", err);
            None
        }
    }
}

fn handle_inspect_impl(peers: &HashMap<u64, Arc<Peer<MockConn, MockAddr>>>) -> Result<()> {
    let addr = read_line("Which peer?").unwrap();
    if addr == "all" {
        for peer in peers.values() {
            println!("{}", peer);
        };
    } else {
        let addr = addr.parse::<u64>()?;
        let peer = peers.get(&addr).ok_or("Peer not in network")?;
        println!("{}", peer);
    }
    Ok(())
}

fn handle_inspect(peers: &HashMap<u64, Arc<Peer<MockConn, MockAddr>>>) {
    if let Err(e) = handle_inspect_impl(peers) {
        println!("{}", e);
    }
}

fn handle_quit() {}

fn handle_help() {
    println!("Commands:");
    for (c, help) in COMMANDS.iter() {
        println!("{}: {}", c, help);
    }
}

fn handle_put() {
    if let Err(e) = handle_put_impl() {
        println!("{}", e);
    }
}

fn handle_put_impl() -> Result<()> {
    let key = read_line("Enter a key").unwrap();
    let value = read_line("Enter a value").unwrap();
    let addr = read_line("On which peer?").unwrap();
    let peer = MockAddr::ApiAddr(addr.parse::<u64>()?);
    let len = std::cmp::min(32, key.len());
    let mut raw_key = [0; 32];

    raw_key[..len].copy_from_slice(&key.as_bytes()[..len]);

    let dht_put = DhtPut {
        ttl: 10,
        replication: 2,
        key: raw_key,
        value: value.as_bytes().to_vec(),
    };

    let mut con = MockConn::open(peer, 1000)?;
    con.send(Message::DhtPut(dht_put))?;

    println!("Sent a DHT PUT message to {:?}", peer);
    Ok(())
}

fn handle_get() {
    if let Err(e) = handle_get_impl() {
        println!("{}", e);
    }
}

fn handle_get_impl() -> Result<()> {
    let key = read_line("Enter a key").unwrap();
    let peer = MockAddr::ApiAddr(read_line("On which peer?").unwrap().parse::<u64>()?);

    let len = std::cmp::min(32, key.len());

    let mut raw_key = [0; 32];
    raw_key[..len].copy_from_slice(&key.as_bytes()[..len]);

    let dht_get = DhtGet { key: raw_key };

    let mut con = MockConn::open(peer, 1000)?;
    con.send(Message::DhtGet(dht_get))?;

    match con.receive()? {
        Message::DhtSuccess(dht_success) => {
            let key = std::str::from_utf8(&dht_success.key)?;
            let value = std::str::from_utf8(&dht_success.value)?;
            println!("Received value for key {}:\n\n{}", key, value);
        }
        Message::DhtFailure(dht_failure) => {
            let key = std::str::from_utf8(&dht_failure.key)?;
            println!("Failed to retrieve value for key {}", key);
        }
        msg => eprintln!("Unexpected message of type {}", msg),
    }
    Ok(())
}


fn handle_join(peers: &mut HashMap<u64, Arc<Peer<MockConn, MockAddr>>>) {
    if let Err(e) = handle_join_impl(peers) {
        println!("{}", e);
    }
}

fn handle_join_impl(peers: &mut HashMap<u64, Arc<Peer<MockConn, MockAddr>>>) -> Result<()> {
    let addr = read_line("Address of the new peer?").unwrap().parse::<u64>()?;
    let bootstrap = read_line("Address of the bootstrap peer?").unwrap().parse::<u64>()?;

    let peer = join(addr, bootstrap);
    peers.insert(addr, peer);
    Ok(())
}

fn handle_consistency(peers: &HashMap<u64, Arc<Peer<MockConn, MockAddr>>>) {
    let peers: Vec<Arc<Peer<_,_>>> = peers.values().map(|x| x.clone()).collect();;
    Peer::preds_consistent(peers);
}