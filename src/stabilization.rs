//! This module is responsible for creating and updating the finger table needed for routing.
//!
//! The [`Stabilization`] struct should be used in regular intervals to make sure that new peers
//! joining the network are recognized and added to the finger table.
//!
//! [`Stabilization`]: struct.Stabilization.html

use crate::network::ConnectionTrait;
use crate::network::PeerAddr;
use crate::procedures::Procedures;
use crate::routing::identifier::*;
use crate::routing::Routing;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::Mutex;

/// Basic information needed to connect to the network using a bootstrap peer
pub struct Bootstrap<C, A>
    where
        C: ConnectionTrait<Address=A>,
        A: PeerAddr,
{
    current_addr: A,
    boot_addr: A,
    fingers: usize,
    p: PhantomData<C>,
}

impl<A: PeerAddr, C: ConnectionTrait<Address=A>> Bootstrap<C, A> {
    /// Initializes the bootstrap algorithm by providing the peer's own address,
    /// the address of a bootstrapping peer and the number of fingers that
    /// should be stored.
    pub fn new(current_addr: A, boot_addr: A, fingers: usize) -> Self {
        Self {
            current_addr,
            boot_addr,
            fingers,
            p: PhantomData,
        }
    }

    /// Creates a new routing table by asking the bootstrap peer for all relevant information.
    ///
    /// This first finds the peer which is currently responsible for our identifier range and
    /// will become our successor. After that we obtain the current predecessor of that peer
    /// and set it as our predecessor which also updates the predecessor information of the
    /// successor peer. Finally, we initialize the finger table with our own address.
    pub fn bootstrap(&self, timeout: u64) -> crate::Result<Routing<A>> {
        let procedures: Procedures<C, A> = Procedures::new(timeout);
        let current_id = self.current_addr.identifier();

        let successor = procedures.find_peer(current_id, self.boot_addr)?;
        let predecessor = &self.current_addr;
        let finger_table = vec![self.current_addr; self.fingers];

        Ok(Routing::new(
            self.current_addr,
            *predecessor,
            successor,
            finger_table,
            true,
        ))
    }
}

/// Stabilize the [`Routing`] table in regular intervals
///
/// [`Routing`]: ../routing/struct.Routing.html
pub struct Stabilization<C, A>
    where
        C: ConnectionTrait<Address=A>,
        A: PeerAddr,
{
    procedures: Procedures<C, A>,
    routing: Arc<Mutex<Routing<A>>>,
}

impl<C: ConnectionTrait<Address=A>, A: PeerAddr> Stabilization<C, A> {
    /// Initializes the stabilization struct with a routing object and the connection timeout.
    pub fn new(routing: Arc<Mutex<Routing<A>>>, timeout: u64) -> Self {
        let procedures = Procedures::new(timeout);

        Self {
            procedures,
            routing,
        }
    }

    /// Updates the successor and finger tables
    ///
    /// The current successor is asked for its predecessor. If the predecessor would be a closer
    /// successor than the field in the routing struct is updated.
    ///
    /// After that the finger tables are updated by iterating through each entry and finding the
    /// peer responsible for that finger.
    pub fn stabilize(&mut self) -> crate::Result<()> {
        info!("Stabilizing routing information");

        self.check_predecessor();

        let update_successors = self.update_successors();
        let update_fingers = self.update_fingers();
        let routing = self.routing.lock().unwrap();

        debug!("Current routing information:\n\n{:#?}", *routing);
        update_successors.and(update_fingers)
    }


    fn update_successors(&self) -> crate::Result<()> {
        let (current, successors) = {
            let routing = self.routing.lock().unwrap();
            (routing.current, routing.successor.clone())
        };

        let new_successors: Vec<_> = successors
            .iter()
            .map(|succ| self.procedures.get_successors(*current, **succ))
            .filter_map(Result::ok)
            .next()
            .unwrap_or(Vec::new());

        if let Some(succ) = new_successors.first() {
            self.procedures.notify(*current, **succ);
        }

        let ids_old = successors.iter().map(|i| **i).collect();
        let ids_new = new_successors.iter().map(|i| **i).collect();
        self.procedures.send_successor_changes(*current, ids_old, ids_new);

        self.routing.lock().unwrap().set_successors(new_successors);
        Ok(())
    }

    fn update_fingers(&self) -> crate::Result<()> {
        let (current, fingers) = {
            let routing = self.routing.lock().unwrap();

            (routing.current, routing.fingers())
        };

        info!("Update fingers");

        for i in 0..fingers {
            // TODO do not hardcode for 256 bits here
            let identifier = current.identifier() + Identifier::with_bit(255 - i);
            // FIXME: do not send message to current
            let peer = self.procedures.find_peer(identifier, *current)?;

            let mut routing = self.routing.lock().unwrap();
            routing.set_finger(i, peer);
        }

        Ok(())
    }

    fn check_predecessor(&self) {
        if self.predecessor_failed() {
            self.routing.lock().unwrap().set_predecessor_failed();
        }
    }

    fn predecessor_failed(&self) -> bool {
        if self.routing.lock().unwrap().get_predecessor_failed() {
            true
        } else if C::open(*self.routing.lock().unwrap().predecessor, 3600).is_err() {
            self.routing.lock().unwrap().set_predecessor_failed();
            true
        } else {
            false
        }
    }
}