extern crate typed_arena;

use std::cell::Cell;
use std::mem;

use super::{Reap, Rp};

struct DropTracker<'a>(&'a Cell<usize>);
impl<'a> Drop for DropTracker<'a> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

struct Node<'a>(Option<Rp<Node<'a>>>, usize, DropTracker<'a>);

// Simple convenience function for the number of chunks in the given `Reap`.
fn n_chunks<T>(reap: &Reap<T>) -> usize {
    reap.0.chunks.borrow().len()
}

#[test]
fn reap_as_intended() {
    let drop_counter = Cell::new(0);
    let reap = Reap::with_capacity(2);

    let mut node = reap.allocate(Node(None, 1, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 1);

    node = reap.allocate(Node(Some(node), 2, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 1);

    node = reap.allocate(Node(Some(node), 3, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 2);

    node = reap.allocate(Node(Some(node), 4, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 2);


    assert_eq!(node.1, 4);
    assert_eq!(node.0.as_ref().unwrap().1, 3);
    assert_eq!(node.0.as_ref().unwrap().0.as_ref().unwrap().1, 2);
    assert_eq!(node.0.as_ref().unwrap().0.as_ref().unwrap().0.as_ref().unwrap().1,
               1);
    assert!(node.0.as_ref().unwrap().0.as_ref().unwrap().0.as_ref().unwrap().0.is_none());

    mem::drop(node);
    assert_eq!(drop_counter.get(), 4);

    let mut node = reap.allocate(Node(None, 5, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 2);

    for i in 6..11 {
        node = reap.allocate(Node(Some(node), i, DropTracker(&drop_counter)));
        assert_eq!(n_chunks(&reap), 2);
    }

    node = reap.allocate(Node(Some(node), 11, DropTracker(&drop_counter)));
    assert_eq!(n_chunks(&reap), 3);

    assert_eq!(node.1, 11);
    assert_eq!(node.0.as_ref().unwrap().1, 10);
    assert_eq!(node.0.as_ref().unwrap().0.as_ref().unwrap().1, 9);
    assert_eq!(node.0.as_ref().unwrap().0.as_ref().unwrap().0.as_ref().unwrap().1,
               8);
    assert_eq!(node.0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .1,
               7);
    assert_eq!(node.0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .1,
               6);
    assert_eq!(node.0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .0
                   .as_ref()
                   .unwrap()
                   .1,
               5);
    assert!(node.0
        .as_ref()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .0
        .as_ref()
        .unwrap()
        .0
        .is_none());

    mem::drop(node);
    assert_eq!(drop_counter.get(), 11);
}

#[test]
fn test_zero_cap() {
    let reap = Reap::with_capacity(0);
    assert_eq!(n_chunks(&reap), 0);

    let a = reap.allocate(1);
    let b = reap.allocate(2);
    assert_eq!(*a, 1);
    assert_eq!(*b, 2);
}

struct ZeroSized;

#[test]
fn test_zero_sized_type() {
    let reap = Reap::new();
    let mut v = Vec::with_capacity(100);

    for _ in 0..100 {
        let alloc = reap.allocate(ZeroSized);
        v.push(alloc);
    }
    assert_eq!(n_chunks(&reap), 0);
}
