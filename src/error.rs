//! Custom error types
//!
//! The [`MessageError`] is currently the only custom error type. It can be used
//! when an unexpected message has been received.
//!
//! [`MessageError`]: struct.MessageError.html

use crate::message::Message;
use crate::network::PeerAddr;
use std::error::Error;
use std::fmt;

/// Error type to use when an unexpected message has been received
///
/// A specific [`Message`] object needs to be available for this error type.
/// If no valid message has been received yet, one should use a different
/// error type like [`io::Error`].
///
/// [`Message`]: message/enum.Message.html
/// [`io::Error`]: ../std/io/struct.Error.html
#[derive(Debug)]
pub struct MessageError<A> {
    msg: Message<A>,
}

impl<A: PeerAddr> MessageError<A> {
    /// Creates a new message error from an existing message as well as an
    /// arbitrary error payload.
    ///
    /// # Examples
    ///
    /// ```
    /// # use dht::error::MessageError;
    /// # use dht::message::{Message, p2p};
    /// use std::net::SocketAddr;
    /// # let socket_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// # let msg = Message::PredecessorNotify(p2p::PredecessorNotify { socket_addr });
    /// #
    /// let result = if let Message::DhtSuccess(_) = msg {
    ///     Ok("yay")
    /// } else {
    ///     Err(MessageError::new(msg))
    /// };
    /// ```
    pub fn new(msg: Message<A>) -> Self {
        MessageError { msg }
    }
}

impl<A: PeerAddr> fmt::Display for MessageError<A> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Unexpected message type {}", self.msg)
    }
}

impl<A: PeerAddr> Error for MessageError<A> {
    fn description(&self) -> &str {
        "Unexpected message type"
    }

    fn cause(&self) -> Option<&dyn Error> {
        None
    }
}
