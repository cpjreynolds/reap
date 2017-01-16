extern crate typed_arena;
extern crate test;
extern crate rand;

use std::cell::Cell;
use std::mem;

use self::typed_arena::Arena;
use self::test::Bencher;
use self::rand::Rand;

use super::{Reap, Rp};


// Simple convenience function for the number of chunks in the given `Reap`.
fn n_chunks<T>(reap: &Reap<T>) -> usize {
    reap.0.chunks.borrow().len()
}

#[test]
fn reap_as_intended() {
    struct DropTracker<'a>(&'a Cell<usize>);
    impl<'a> Drop for DropTracker<'a> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct Node<'a>(Option<Rp<Node<'a>>>, usize, DropTracker<'a>);

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

#[test]
fn test_zero_sized_type() {
    struct ZeroSized;

    let reap = Reap::new();
    let mut v = Vec::with_capacity(100);

    for _ in 0..100 {
        let alloc = reap.allocate(ZeroSized);
        v.push(alloc);
    }
    assert_eq!(n_chunks(&reap), 0);
}

// Before you look at these benchmarks, please be advised that I have absolutely zero experience
// writing benchmarks, and the following are just my best effort.
//
// That being said, if someone with more experience could help improve these that would be
// wonderful.

#[bench]
fn bench_reap_copy(b: &mut Bencher) {
    const CAP: usize = 8192;

    let reap = Reap::new();
    let mut storage = Vec::with_capacity(CAP);

    b.iter(|| {
        for i in 0..CAP {
            let alloc = reap.allocate(i);
            storage.push(alloc);
        }

        for i in (0..(CAP - (CAP / 4))).filter(|n| n % 4 == 0) {
            storage.swap_remove(i);
        }

        for i in 0..(CAP / 4) {
            let alloc = reap.allocate(i);
            storage.push(alloc);
        }
    });
}

#[bench]
fn bench_arena_copy(b: &mut Bencher) {
    const CAP: usize = 8192;

    let arena = Arena::new();
    let mut storage = Vec::with_capacity(CAP);

    b.iter(|| {
        for i in 0..CAP {
            let alloc = arena.alloc(i);
            storage.push(alloc);
        }

        for i in (0..(CAP - (CAP / 4))).filter(|n| n % 4 == 0) {
            storage.swap_remove(i);
        }

        for i in 0..(CAP / 4) {
            let alloc = arena.alloc(i);
            storage.push(alloc);
        }
    });
}

#[bench]
fn bench_box_copy(b: &mut Bencher) {
    const CAP: usize = 8192;

    let mut storage = Vec::with_capacity(CAP);

    b.iter(|| {
        for i in 0..CAP {
            let alloc = Box::new(i);
            storage.push(alloc);
        }

        for i in (0..(CAP - (CAP / 4))).filter(|n| n % 4 == 0) {
            storage.swap_remove(i);
        }

        for i in 0..(CAP / 4) {
            let alloc = Box::new(i);
            storage.push(alloc);
        }
    });
}
