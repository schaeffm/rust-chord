//! This modules implements identifier-based routing using consistent hashing.
//!
//! An [`Identifier`] is a 256 bit number on an identifier circle containing
//! all non-negative integers module 2^256. Each peer can obtain its identifier
//! by hashing its own ip address. The peers are responsible for the range on
//! the identifier circle beginning with the identifier after their predecessor
//! up to and including their own identifier.
//!
//! The [`Routing`] struct supports all operations to obtain the closest known
//! peer on the identifier circle to some given identifier by using a so called
//! "finger table". This data structure contains pointers to the peers
//! responsible for every 2^i-th identifier after our own identifier. This
//! allows us to find the responsible peer for an arbitrary identifier in
//! O(log(N)) steps where N is the size of the whole network.
//!
//! [`Identifier`]: identifier/struct.Identifier.html
//! [`Routing`]: struct.Routing.html

use self::identifier::*;

pub mod identifier;

/// This struct stores routing information about other peers.
#[derive(Debug, Clone)]
pub struct Routing<T> {
    pub current: IdentifierValue<T>,
    // TODO should maybe be an Option
    pub predecessor: Option<IdentifierValue<T>>,
    // TODO use BinaryHeap for multiple successors
    pub successor: Vec<IdentifierValue<T>>,
    // TODO
    pub finger_table: Vec<IdentifierValue<T>>,
}

impl<T: Identify + Copy + Clone> Routing<T> {
    /// Creates a new `Routing` instance for the given initial values.
    pub fn new(current: T, predecessor: Option<T>, successor: T, finger_table: Vec<T>) -> Self {
        let predecessor = match predecessor {
            Some(p) => Some(IdentifierValue::new(p)),
            None => None,
        };

        Self {
            current: IdentifierValue::new(current),
            predecessor: predecessor,
            successor: vec![IdentifierValue::new(successor)],
            finger_table: finger_table.into_iter().map(IdentifierValue::new).collect(),
        }
    }

    /// Sets the predecessor's address.
    pub fn set_predecessor(&mut self, new_pred: Option<T>) {
        self.predecessor = new_pred.map(IdentifierValue::new);
    }

    pub fn set_successors(&mut self, new_succs: Vec<T>) {
        self.successor = new_succs.into_iter().map(IdentifierValue::new).collect();
        if let Some(successor) = self.successor.first() {
            let diff = successor.identifier() - self.current.identifier();

            for i in diff.leading_zeros() as usize..self.finger_table.len() {
                self.finger_table[i] = *successor;
            }
        }
    }

    /// Sets the finger for the given index.
    pub fn set_finger(&mut self, index: usize, finger: T) {
        self.finger_table[index] = IdentifierValue::new(finger);
    }

    /// Returns the number of fingers.
    pub fn fingers(&self) -> usize {
        self.finger_table.len()
    }

    /// Returns the peer closest to the given identifier.
    pub fn closest_preceding_peer(&self, identifier: Identifier) -> &IdentifierValue<T> {
        for finger in &self.finger_table {
            if finger.identifier().is_between(&self.current.identifier(), &identifier) {
                return finger
            }
        }
        return self.successor.first().unwrap();
    }

    pub fn preds_consistent(_peers: Vec<Self>) -> bool {
        /*
        let len = peers.len();
        peers.sort_by(|a, b| a.current.identifier().cmp(&b.current.identifier()));
        // rotate ids by one
        let ids = peers.iter().map(|x| x.current.identifier()).cycle().skip(1).take(len);
        let preds = peers.iter().map(|x| x.predecessor.identifier());

        ids.eq(preds)
        */

        false
    }
}

/// Returns the finger table entry number for the closest predecessor.
fn _finger_table_entry_number(current: Identifier, lookup: Identifier) -> u8 {
    let diff = lookup - current - Identifier::new_from_usize(1);
    let zeros = diff.leading_zeros();

    255 - zeros as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use bigint::U256;

    #[test]
    fn return_first_entry() {
        let mut bytes1 = [0; 32];
        let mut bytes2 = [0; 32];

        U256::from(0).to_big_endian(&mut bytes1);
        U256::from(2).to_big_endian(&mut bytes2);

        let id1 = Identifier::new(&bytes1);
        let id2 = Identifier::new(&bytes2);

        assert_eq!(finger_table_entry_number(id1, id2), 0);
    }

    #[test]
    fn return_second_entry() {
        let mut bytes1 = [0; 32];
        let mut bytes2 = [0; 32];

        U256::from(0).to_big_endian(&mut bytes1);
        U256::from(3).to_big_endian(&mut bytes2);

        let id1 = Identifier::new(&bytes1);
        let id2 = Identifier::new(&bytes2);

        assert_eq!(finger_table_entry_number(id1, id2), 1);
    }

    #[test]
    fn return_last_entry() {
        let mut bytes1 = [0; 32];
        let mut bytes2 = [0; 32];

        U256::from(1).to_big_endian(&mut bytes1);
        U256::from(0).to_big_endian(&mut bytes2);

        let id1 = Identifier::new(&bytes1);
        let id2 = Identifier::new(&bytes2);

        assert_eq!(finger_table_entry_number(id1, id2), 255);
    }
}
