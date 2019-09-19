//! Networking abstraction layer for TCP connections
//!
//! This module provides some nice abstraction from raw TCP sockets to
//! connections allowing to send and receive [`Message`] objects.
//! Furthermore, it includes parallel handling of incoming connections using
//! a thread pool and the abstraction of handlers.
//!
//! [`Message`]: ../message/enum.Message.html

use crate::message::Message;
use crate::routing::identifier::Identify;
use crate::Result;
use std::io;
use std::io::prelude::*;
use std::io::Cursor;
use std::net::*;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use threadpool::ThreadPool;

const MAX_MESSAGE_SIZE: usize = 64000;

/// A connection between two peers to send Message objects via TCP
///
/// # Examples
///
/// ```no_run
/// # use dht::network::Connection;
/// #
/// let mut con = Connection::open("127.0.0.1:8080", 3600)
///     .expect("Could not open connection");
///
/// let msg = con.receive().expect("could not receive message");
/// con.send(&msg).expect("could not send message");
/// ```
pub trait PeerAddr:
    Clone + Copy + std::fmt::Display + std::fmt::Debug + std::cmp::PartialEq + Send + Identify + 'static + std::cmp::Eq + std::hash::Hash
{
}
impl<T> PeerAddr for T where
    T: Clone
        + Copy
        + std::fmt::Display
        + std::fmt::Debug
        + std::cmp::PartialEq
        + Send
        + Identify
        + 'static
        + std::hash::Hash
        + std::cmp::Eq
{
}
pub trait ConnectionTrait
where
    Self: Sized + Send + 'static,
{
    type Address: PeerAddr;
    type Listener: Send;
    fn open(addr: Self::Address, timeout_ms: u64) -> Result<Self>;
    fn receive(&mut self) -> Result<Message<Self::Address>>;
    fn send(&mut self, msg: Message<Self::Address>) -> Result<()>;
    //fn peer_addr(&self) -> Result<Self::Address>;
    //fn local_addr(&self) -> Result<Self::Address>;
    fn shutdown(self) -> Result<()>;
    fn bind(addr: Self::Address) -> Result<Self::Listener>;
    fn listen<'a>(listener: &'a Self::Listener) -> Box<dyn 'a + Iterator<Item = Self>>;
}

impl ConnectionTrait for Connection {
    type Address = SocketAddr;
    type Listener = TcpListener;
    fn open(addr: SocketAddr, timeout_ms: u64) -> Result<Self> {
        let stream = TcpStream::connect(addr)?;

        trace!("Connection to {} - Opened", stream.peer_addr()?);

        let timeout = Duration::from_millis(timeout_ms);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;

        Ok(Self::from_stream(stream))
    }

    fn receive(&mut self) -> Result<Message<Self::Address>> {
        // read bytes from tcp stream
        let size = self.stream.read(self.buffer.as_mut())?;

        // create cursor to parse message
        let msg = Message::parse(Cursor::new(&self.buffer[..size]))?;

        // output debug information
        trace!(
            "Connection to {} - Received message of type {}",
            self.stream.peer_addr()?,
            msg
        );

        Ok(msg)
    }

    fn send(&mut self, msg: Message<Self::Address>) -> Result<()> {
        // create cursor to write message
        let size = msg.write_to(Cursor::new(self.buffer.as_mut()))?;

        // output debug information
        trace!(
            "Connection to {} - Sent message of type {}",
            self.stream.peer_addr()?,
            msg
        );

        // write bytes to tcp stream
        Ok(self.stream.write_all(&self.buffer[..size])?)
    }

    //fn peer_addr(&self) -> Result<SocketAddr> {
    //    Ok(self.stream.peer_addr()?)
    //}

    fn shutdown(self) -> Result<()> {
        Ok(self.stream.shutdown(Shutdown::Both)?)
    }

    fn listen<'a>(listener: &'a Self::Listener) -> Box<dyn 'a + Iterator<Item = Self>> {
        Box::new(
            listener
                .incoming()
                .filter_map(std::result::Result::ok)
                .map(Connection::from_stream),
        )
    }

    fn bind(addr: Self::Address) -> Result<Self::Listener> {
        Ok(TcpListener::bind(addr)?)
    }
}

pub struct Connection {
    stream: TcpStream,
    buffer: [u8; MAX_MESSAGE_SIZE],
}

impl Connection {
    /// Opens a TCP connection to a remote peer.
    ///
    /// This uses [`TcpStream::connect`] to create a new TCP socket to the
    /// remote peer with address `addr`.
    ///
    /// `timeout_ms` is the timeout in milliseconds for both read and write
    /// operations. See [`TcpStream::set_read_timeout`] and
    /// [`TcpStream::set_write_timeout`] for further documentation.
    ///
    /// [`TcpStream::connect`]:
    /// ../../std/net/struct.TcpStream.html#method.connect
    /// [`TcpStream::set_read_timeout`]:
    /// ../../std/net/struct.TcpStream.html#method.set_read_timeout
    /// [`TcpStream::set_write_timeout`]:
    /// ../../std/net/struct.TcpStream.html#method.set_write_timeout
    pub fn open<A: ToSocketAddrs>(addr: A, timeout_ms: u64) -> io::Result<Self> {
        // TODO add connection timeout
        let stream = TcpStream::connect(addr)?;

        trace!("Connection to {} - Opened", stream.peer_addr()?);

        let timeout = Duration::from_millis(timeout_ms);
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;

        Ok(Self::from_stream(stream))
    }

    fn from_stream(stream: TcpStream) -> Self {
        // TODO set read and write timeout
        let buffer = [0; MAX_MESSAGE_SIZE];
        Self { stream, buffer }
    }

    /// Receives a message from the remote peer.
    ///
    /// This operation is blocking until a message has been received.
    pub fn receive(&mut self) -> io::Result<Message<SocketAddr>> {
        // read bytes from tcp stream
        let size = self.stream.read(self.buffer.as_mut())?;

        // create cursor to parse message
        let msg = Message::parse(Cursor::new(&self.buffer[..size]))?;

        // output debug information
        trace!(
            "Connection to {} - Received message of type {}",
            self.stream.peer_addr()?,
            msg
        );

        Ok(msg)
    }

    /// Sends a message to the remote peer.
    ///
    /// This operation is blocking until the message has been sent.
    pub fn send(&mut self, msg: &Message<SocketAddr>) -> io::Result<()> {
        // create cursor to write message
        let size = msg.write_to(Cursor::new(self.buffer.as_mut()))?;

        // output debug information
        trace!(
            "Connection to {} - Sent message of type {}",
            self.stream.peer_addr()?,
            msg
        );

        // write bytes to tcp stream
        self.stream.write_all(&self.buffer[..size])
    }

    /// Returns the socket address of the remote peer of this TCP connection.
    ///
    /// See [`TcpStream::peer_addr`] for further documentation.
    ///
    /// [`TcpStream::peer_addr`]:
    /// ../../std/net/struct.TcpStream.html#method.peer_addr
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// Returns the socket address of the local half of this TCP connection.
    ///
    /// See [`TcpStream::local_addr`] for further documentation.
    ///
    /// [`TcpStream::local_addr`]:
    /// ../../std/net/struct.TcpStream.html#method.local_addr
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    /// Shuts down the read and write part of this connection.
    ///
    /// See [`TcpStream::shutdown`] for further documentation.
    ///
    /// [`TcpStream::shutdown`]:
    /// ../../std/net/struct.TcpStream.html#method.shutdown
    pub fn shutdown(&mut self) -> io::Result<()> {
        self.stream.shutdown(Shutdown::Both)
    }
}

/// A trait to handle incoming requests from a [`Server`].
///
/// The methods [`handle_connection`] and [`handle_error`] are called based on
/// the success of the incoming request.
///
/// [`Server`]: struct.Server.html
/// [`handle_connection`]: #tymethod.handle_connection
/// [`handle_error`]: #tymethod.handle_error
pub trait ServerHandler<C> {
    /// A connection has been established with some remote peer.
    ///
    /// The handler can exchange messages with the peer via the given
    /// `connection` object.
    fn handle_connection(&self, connection: C);

    /// The incoming request was unsuccessful and an error was raised.
    ///
    /// The given `error` should be handled appropiately.
    fn handle_error(&self, error: io::Error);

    //// Handles an incomming connection.
    ////
    //// Depending on the `result` this either calls [`handle_error`] or
    //// creates a new [`Connection`] from the given [`TcpStream`] and
    //// calls [`handle_connection`].
    ////
    //// [`handle_error`]: #tymethod.handle_error
    //// [`Connection`]: struct.Connection.html
    //// [`TcpStream`]: ../../std/net/struct.TcpStream.html
    //// [`handle_connection`]: #tymethod.handle_connection
    //fn handle_incoming(&self, result: io::Result<TcpStream>) {
    //    match result {
    //        Ok(stream) => {
    //            trace!(
    //                "Handling incoming connection from {}",
    //                stream.peer_addr().unwrap()
    //            );

    //            // TODO handle timeouts
    //            let connection = Connection::from_stream(stream);

    //            self.handle_connection(connection)
    //        }
    //        Err(error) => self.handle_error(error),
    //    }
    //}
}

/// A multithreaded server waiting for connections
///
/// # Examples
///
/// ```no_run
/// # use dht::network::*;
/// # use std::io;
/// #
/// # struct TestHandler;
/// # impl ServerHandler<Connection> for TestHandler {
/// #     fn handle_connection(&self, _: Connection) {}
/// #     fn handle_error(&self, _: io::Error) {}
/// # }
/// #
/// # let handler = TestHandler;
/// #
/// let server = Server::new(handler);
///
/// server.listen("127.0.0.1:8080".parse().unwrap(), 4)
///     .expect("could not bind to port");
/// ```
pub struct Server<T> {
    handler: Arc<T>,
}

impl<T: Send + Sync + 'static> Server<T> {
    /// Creates a new server for the given handler.
    ///
    /// The [`ServerHandler`] must also implement [`Send`] and [`Sync`] to
    /// ensure it can be shared between threads.
    ///
    /// [`ServerHandler`]: trait.ServerHandler.html
    /// [`Send`]: ../../std/marker/trait.Send.html
    /// [`Sync`]: ../../std/marker/trait.Sync.html
    pub fn new(handler: T) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }

    pub fn from_arc(handler: &Arc<T>) -> Self {
        Self {
            handler: Arc::clone(handler),
        }
    }

    /// Listens on the given socket address.
    ///
    /// `num_workers` defines the number of worker threads which handle
    /// incoming requests in parallel.
    pub fn listen<A: PeerAddr, C: ConnectionTrait<Address = A>>(
        self,
        addr: A,
        num_workers: usize,
    ) -> Result<thread::JoinHandle<()>>
    where
        T: ServerHandler<C>,
    {
        let listener = C::bind(addr)?;

        //trace!("Server listening on address {}", listener.local_addr()?);

        let handle = thread::spawn(move || {
            let pool = ThreadPool::new(num_workers);

            for con in C::listen(&listener) {
                let handler = Arc::clone(&self.handler);
                pool.execute(move || {
                    handler.handle_connection(con);
                });
            }
        });

        Ok(handle)
    }
}
