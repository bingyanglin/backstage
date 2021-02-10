use std::{collections::HashMap, hash::Hash};

use anymap::{any::Any, Map};

use crate::{AddressBook, Mailbox};

pub type Address = usize;

#[derive(Default)]
pub struct Registry<A: Hash + PartialEq + Eq> {
    gen: usize,
    map: HashMap<A, Map<dyn Any + Send>>,
}

impl Registry<Address> {
    fn get_map(&self, id: <Self as AddressBook>::Identifier) -> Option<&Map<dyn Any + Send>> {
        self.map.get(&id)
    }

    fn add_to<M: 'static + Mailbox + Send>(&mut self, id: <Self as AddressBook>::Identifier, addressable: M) {
        self.map.entry(id).or_insert(Map::new()).insert(addressable);
    }
}

impl AddressBook for Registry<Address> {
    type Identifier = Address;

    fn insert<M: 'static + Mailbox + Send>(&mut self, addressable: M) -> Self::Identifier {
        let id = self.gen;
        self.map.entry(id).or_insert(Map::new()).insert(addressable);
        self.gen += 1;
        id
    }

    fn get<M: 'static + Mailbox + Send>(&self, id: Self::Identifier) -> Option<&M> {
        self.map.get(&id).and_then(|anymap| anymap.get::<M>())
    }

    fn remove<M: 'static + Mailbox + Send>(&mut self, id: Self::Identifier) -> Option<M> {
        self.map.get_mut(&id).and_then(|anymap| anymap.remove::<M>())
    }
}
