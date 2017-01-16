#![cfg_attr(test, feature(test))]

use std::cell::{RefCell, Cell};
use std::rc::Rc;
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::mem;
use std::cmp::{self, Ordering};
use std::marker;
use std::hash::{self, Hash};
use std::fmt;
use std::borrow;

#[cfg(test)]
mod test;

// Default initial capacity in bytes.
const PAGE: usize = 4096;

// A `Chunk` represents a single contiguous allocation within the `Reap`.
//
// TODO: If/when `RawVec` is stabilized, use it instead of a raw pointer and capacity. Or just find
// a better alternative.
struct Chunk<T> {
    // Pointer to the allocation. `heap::EMPTY` (1 as *mut T) for ZSTs.
    ptr: *mut T,
    // Capacity of the allocation. `!0` (usize::MAX) for ZSTs.
    cap: usize,
}

impl<T> Chunk<T> {
    // Creates a new `Chunk` with the given `capacity`.
    #[inline]
    fn new(capacity: usize) -> Chunk<T> {
        let mut v = Vec::with_capacity(capacity);
        let ptr = v.as_mut_ptr();
        // We have all the information necessary to take ownership of `Vec`'s allocation and
        // reconstitute it later.
        mem::forget(v);

        Chunk {
            ptr: ptr,
            cap: capacity,
        }
    }

    // Returns a pointer to the start of the allocated space.
    #[inline]
    fn start(&self) -> *mut T {
        self.ptr
    }

    // Returns a pointer to the end of the allocated space.
    #[inline]
    fn end(&self) -> *mut T {
        unsafe {
            if mem::size_of::<T>() == 0 {
                // A pointer as large as possible for ZSTs.
                !0 as *mut T
            } else {
                self.start().offset(self.capacity() as isize)
            }
        }
    }

    // Returns the capacity of the `Chunk`.
    #[inline]
    fn capacity(&self) -> usize {
        if mem::size_of::<T>() == 0 {
            !0
        } else {
            self.cap
        }
    }
}

impl<T> Drop for Chunk<T> {
    fn drop(&mut self) {
        // Give the allocation back to `Vec` so that it may be deallocated.
        //
        // Since calling `Drop::drop` for individual elements within a `Chunk` is handled by `Rp`,
        // and a `Chunk` will not be dropped until its owning `Reap` is, which in turn will not
        // be dropped until its refcount is zero, it is guaranteed that when a `Chunk` is dropped,
        // destructors have already run on all appropriate elements in its allocation.
        //
        // That was a lot of words, I hope they made as much sense to you as they did to me.
        unsafe {
            Vec::from_raw_parts(self.ptr, 0, self.cap);
        }
    }
}

pub struct Reap<T>(Rc<InnerReap<T>>);

// This struct is a necessary evil for `Rc`'s purposes; it is always kept behind an `Rc`.
struct InnerReap<T> {
    // Pointer to the next object to be allocated. (If the freelist is empty).
    ptr: Cell<*mut T>,
    // Pointer to the end of the current `Chunk`, when this pointer is reached a new `Chunk` is
    // allocated.
    end: Cell<*mut T>,
    // Reap chunks, each double the size of the last.
    chunks: RefCell<Vec<Chunk<T>>>,
    // Stack of pointers to memory locations able to be reused.
    freelist: RefCell<Vec<*mut T>>,
}

impl<T> Reap<T> {
    /// Creates a new `Reap<T>`.
    #[inline]
    pub fn new() -> Reap<T> {
        Reap(Rc::new(InnerReap {
            // Set both `ptr` and `end` to 0 so that the first call to `allocate()` will trigger a
            // `grow()`
            ptr: Cell::new(0 as *mut T),
            end: Cell::new(0 as *mut T),
            chunks: RefCell::new(Vec::new()),
            freelist: RefCell::new(Vec::new()),
        }))
    }

    pub fn with_capacity(capacity: usize) -> Reap<T> {
        if capacity == 0 {
            Reap::new()
        } else {
            let chunk = Chunk::new(capacity);
            Reap(Rc::new(InnerReap {
                ptr: Cell::new(chunk.start()),
                end: Cell::new(chunk.end()),
                chunks: RefCell::new(vec![chunk]),
                freelist: RefCell::new(Vec::new()),
            }))
        }
    }

    #[inline]
    pub fn allocate(&self, object: T) -> Rp<T> {
        unsafe {
            // First, deal with ZSTs.
            if mem::size_of::<T>() == 0 {
                // Bump our imaginary pointer.
                self.0.ptr.set((self.0.ptr.get() as *mut u8).offset(1) as *mut T);
                // `heap::EMPTY` is unstable so this will have to do.
                let ptr = 1 as *mut T;
                // Don't drop the object, this `ptr::write` is equivalent to `mem::forget`.
                ptr::write(ptr, object);
                Rp::from_raw(ptr, self.clone())
            } else {
                // Reaching this branch means we're not dealing with a ZST, on with the fun stuff.
                //
                // First, check the freelist.
                if let Some(loc) = self.0.freelist.borrow_mut().pop() {
                    ptr::write(loc, object);
                    Rp::from_raw(loc, self.clone())
                } else {
                    // No dice on the freelist, now we act like a normal arena.
                    if self.0.ptr == self.0.end {
                        self.grow()
                    }
                    let ptr = self.0.ptr.get();
                    self.0.ptr.set(self.0.ptr.get().offset(1));
                    ptr::write(ptr, object);
                    Rp::from_raw(ptr, self.clone())
                }
            }
        }
    }

    // Deallocate the given raw pointer.
    //
    // This function is only called by an associated `Rp<T>`'s destructor, which guarantees that
    // the given `ptr` is valid, and actually part of an allocation owned by this `Reap<T>`.
    #[inline]
    fn deallocate(&self, ptr: *mut T) {
        unsafe {
            ptr::drop_in_place(ptr);
        }
        self.0.freelist.borrow_mut().push(ptr);
    }

    #[inline(never)]
    #[cold]
    fn grow(&self) {
        let mut chunks = self.0.chunks.borrow_mut();
        let new_cap;
        if let Some(last_chunk) = chunks.last_mut() {
            let prev_cap = last_chunk.capacity();
            // If doubling the size of the last allocation causes overflow on a `usize`, we most
            // likely have far, far bigger problems.
            //
            // Something something fail early, fail loudly.
            new_cap = prev_cap.checked_mul(2).expect("capacity overflow");
        } else {
            let elem_size = cmp::max(1, mem::size_of::<T>());
            new_cap = PAGE / elem_size;
        }
        let chunk = Chunk::new(new_cap);
        self.0.ptr.set(chunk.start());
        self.0.end.set(chunk.end());
        chunks.push(chunk);
    }
}

impl<T> Clone for Reap<T> {
    fn clone(&self) -> Self {
        Reap(self.0.clone())
    }

    fn clone_from(&mut self, source: &Self) {
        self.0.clone_from(&source.0);
    }
}

/// Reap smart pointer.
pub struct Rp<T> {
    ptr: *mut T,
    reap: Reap<T>,
    _marker: marker::PhantomData<T>,
}

impl<T> Rp<T> {
    /// Constructs an `Rp` from a raw pointer.
    ///
    /// # Safety
    ///
    /// This function is highly unsafe and can lead to all sorts of laundry-eating bad if its
    /// invariants are not maintained.
    ///
    /// * `ptr` **must** have been previously returned from a call to `Rp::into_raw`.
    /// * `reap` **must** be the same `Reap` that allocated `ptr`.
    ///
    /// # Examples
    ///
    /// ```
    /// use reap::{Reap, Rp};
    ///
    /// let reap = Reap::new();
    ///
    /// let x = reap.allocate(101);
    /// let (x_ptr, r) = Rp::into_raw(x);
    ///
    /// unsafe {
    ///     // Convert back to an `Rp` to prevent leak.
    ///     let x = Rp::from_raw(x_ptr, r);
    ///     assert_eq!(*x, 101);
    ///
    ///     // Further calls to `Rc::from_raw(x_ptr, r)` would be memory unsafe.
    /// }
    ///
    /// // `x` went out of scope above so the memory is considered free, so `x_ptr` is now dangling!
    /// ```
    #[inline]
    pub unsafe fn from_raw(ptr: *mut T, reap: Reap<T>) -> Rp<T> {
        Rp {
            ptr: ptr,
            reap: reap,
            _marker: marker::PhantomData,
        }
    }

    /// Consumes the `Rp`, returning the wrapped pointer and associated `Reap`.
    ///
    /// To avoid a memory leak the pointer must be converted back to an `Rp` using `Rp::from_raw`
    /// with its associated `Reap`.
    ///
    /// # Examples
    ///
    /// ```
    /// use reap::{Reap, Rp};
    ///
    /// let reap = Reap::new();
    ///
    /// let x = reap.allocate(101);
    /// let (x_ptr, r) = Rp::into_raw(x);
    /// assert_eq!(unsafe { *x_ptr }, 101);
    ///
    /// unsafe {
    ///     // Convert back to an `Rp` to prevent leak.
    ///     let x = Rp::from_raw(x_ptr, r);
    ///     assert_eq!(*x, 101);
    /// }
    ///
    #[inline]
    pub fn into_raw(mut this: Rp<T>) -> (*mut T, Reap<T>) {
        let ptr = this.ptr;
        // If there is another way to do this someone please tell me, this just feels wrong.
        // I know I could just clone the `Reap` but I'd rather not unnecessarily increment the
        // refcount.
        let reap = unsafe { mem::replace(&mut this.reap, mem::uninitialized()) };
        mem::forget(this);
        (ptr, reap)
    }

    /// Returns a reference to this `Rp<T>`'s associated `Reap<T>`.
    #[inline]
    pub fn reap(&self) -> &Reap<T> {
        &self.reap
    }
}

impl<T> PartialEq for Rp<T>
    where T: PartialEq
{
    #[inline]
    fn eq(&self, other: &Rp<T>) -> bool {
        PartialEq::eq(&**self, &**other)
    }

    #[inline]
    fn ne(&self, other: &Rp<T>) -> bool {
        PartialEq::ne(&**self, &**other)
    }
}

impl<T> PartialOrd for Rp<T>
    where T: PartialOrd
{
    #[inline]
    fn partial_cmp(&self, other: &Rp<T>) -> Option<Ordering> {
        PartialOrd::partial_cmp(&**self, &**other)
    }

    #[inline]
    fn lt(&self, other: &Rp<T>) -> bool {
        PartialOrd::lt(&**self, &**other)
    }

    #[inline]
    fn le(&self, other: &Rp<T>) -> bool {
        PartialOrd::le(&**self, &**other)
    }

    #[inline]
    fn ge(&self, other: &Rp<T>) -> bool {
        PartialOrd::ge(&**self, &**other)
    }

    #[inline]
    fn gt(&self, other: &Rp<T>) -> bool {
        PartialOrd::gt(&**self, &**other)
    }
}

impl<T> Ord for Rp<T>
    where T: Ord
{
    #[inline]
    fn cmp(&self, other: &Rp<T>) -> Ordering {
        Ord::cmp(&**self, &**other)
    }
}

impl<T> Eq for Rp<T> where T: Eq {}

impl<T> Hash for Rp<T>
    where T: Hash
{
    fn hash<H>(&self, state: &mut H)
        where H: hash::Hasher
    {
        (**self).hash(state);
    }
}

impl<T> fmt::Display for Rp<T>
    where T: fmt::Display
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

impl<T> fmt::Debug for Rp<T>
    where T: fmt::Debug
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T> fmt::Pointer for Rp<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Pointer::fmt(&self.ptr, f)
    }
}

impl<I> Iterator for Rp<I>
    where I: Iterator
{
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        (**self).next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<I> DoubleEndedIterator for Rp<I>
    where I: DoubleEndedIterator
{
    fn next_back(&mut self) -> Option<I::Item> {
        (**self).next_back()
    }
}

impl<I> ExactSizeIterator for Rp<I> where I: ExactSizeIterator {}

impl<T> borrow::Borrow<T> for Rp<T> {
    fn borrow(&self) -> &T {
        &**self
    }
}

impl<T> borrow::BorrowMut<T> for Rp<T> {
    fn borrow_mut(&mut self) -> &mut T {
        &mut **self
    }
}

impl<T> AsRef<T> for Rp<T> {
    fn as_ref(&self) -> &T {
        &**self
    }
}

impl<T> AsMut<T> for Rp<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut **self
    }
}

impl<T> Deref for Rp<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.ptr }
    }
}

impl<T> DerefMut for Rp<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.ptr }
    }
}

impl<T> Drop for Rp<T> {
    fn drop(&mut self) {
        self.reap.deallocate(self.ptr)
    }
}
