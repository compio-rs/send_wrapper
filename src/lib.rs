// Copyright 2017 Thomas Keh.
// Copyright 2024 compio-rs
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! This [Rust] library implements a wrapper type called [`SendWrapper`] which
//! allows you to move around non-[`Send`] types between threads, as long as you
//! access the contained value only from within the original thread. You also
//! have to make sure that the wrapper is dropped from within the original
//! thread. If any of these constraints is violated, a panic occurs.
//! [`SendWrapper<T>`] implements [`Send`] and [`Sync`] for any type `T`.
//!
//! # Examples
//!
//! ```rust
//! // This import is important if you want to use deref() or
//! // deref_mut() instead of Deref coercion.
//! use std::{rc::Rc, sync::mpsc::channel, thread};
//!
//! use compio_send_wrapper::SendWrapper;
//!
//! // Rc is a non-Send type.
//! let value = Rc::new(42);
//!
//! // We now wrap the value with `SendWrapper` (value is moved inside).
//! let wrapped_value = SendWrapper::new(value);
//!
//! // A channel allows us to move the wrapped value between threads.
//! let (sender, receiver) = channel();
//!
//! let t = thread::spawn(move || {
//!     // This would panic (because of dereferencing in wrong thread):
//!     // let value = wrapped_value.deref();
//!
//!     // Move SendWrapper back to main thread, so it can be dropped from there.
//!     // If you leave this out the thread will panic because of dropping from wrong thread.
//!     sender.send(wrapped_value).unwrap();
//! });
//!
//! let wrapped_value = receiver.recv().unwrap();
//!
//! // Now you can use the value again.
//! let value = wrapped_value.get().unwrap();
//!
//! let mut wrapped_value = wrapped_value;
//!
//! // You can also get a mutable reference to the value.
//! let value = wrapped_value.get_mut().unwrap();
//! ```
//!
//! # Features
//!
//! This crate has a single feature called `futures` that enables [`Future`] and
//! [`Stream`] implementations for [`SendWrapper`]. You can enable it in
//! `Cargo.toml` like so:
//!
//! ```toml
//! compio-send-wrapper = { version = "...", features = ["futures"] }
//! ```
//!
//! # License
//!
//! `compio-send-wrapper` is distributed under the terms of both the MIT license
//! and the Apache License (Version 2.0).
//!
//! See LICENSE-APACHE.txt, and LICENSE-MIT.txt for details.
//!
//! [Rust]: https://www.rust-lang.org
//! [`Future`]: std::future::Future
//! [`Stream`]: futures_core::Stream
// To build docs locally use `RUSTDOCFLAGS="--cfg docsrs" cargo doc --open --all-features`
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "current_thread_id", feature(current_thread_id))]
#![warn(missing_docs)]

#[cfg(feature = "futures")]
#[cfg_attr(docsrs, doc(cfg(feature = "futures")))]
mod futures;

#[cfg(feature = "current_thread_id")]
use std::thread::current_id;
use std::{
    fmt,
    mem::{self, ManuallyDrop},
    pin::Pin,
    thread::{self, ThreadId},
};

#[cfg(not(feature = "current_thread_id"))]
mod imp {
    use std::{
        cell::Cell,
        thread::{self, ThreadId},
    };
    thread_local! {
        static THREAD_ID: Cell<ThreadId> = Cell::new(thread::current().id());
    }

    pub fn current_id() -> ThreadId {
        THREAD_ID.get()
    }
}

#[cfg(not(feature = "current_thread_id"))]
use imp::current_id;

/// A wrapper which allows you to move around non-[`Send`]-types between
/// threads, as long as you access the contained value only from within the
/// original thread and make sure that it is dropped from within the original
/// thread.
pub struct SendWrapper<T> {
    data: ManuallyDrop<T>,
    thread_id: ThreadId,
}

impl<T> SendWrapper<T> {
    /// Create a `SendWrapper<T>` wrapper around a value of type `T`.
    /// The wrapper takes ownership of the value.
    #[inline]
    pub fn new(data: T) -> SendWrapper<T> {
        SendWrapper {
            data: ManuallyDrop::new(data),
            thread_id: current_id(),
        }
    }

    /// Returns `true` if the value can be safely accessed from within the
    /// current thread.
    #[inline]
    pub fn valid(&self) -> bool {
        self.thread_id == current_id()
    }

    /// Takes the value out of the `SendWrapper<T>`.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    pub unsafe fn take_unchecked(self) -> T {
        // Prevent drop() from being called, as it would drop `self.data` twice
        let mut this = ManuallyDrop::new(self);

        // Safety:
        // - We've just checked that it's valid to access `T` from the current thread
        // - We only move out from `self.data` here and in drop, so `self.data` is
        //   present
        unsafe { ManuallyDrop::take(&mut this.data) }
    }

    /// Takes the value out of the `SendWrapper<T>`.
    ///
    /// # Panics
    ///
    /// Panics if it is called from a different thread than the one the
    /// `SendWrapper<T>` instance has been created with.
    #[track_caller]
    pub fn take(self) -> Option<T> {
        if !self.valid() {
            invalid_deref()
        } else {
            // SAFETY: the same thread as the creator
            Some(unsafe { self.take_unchecked() })
        }
    }

    /// Returns a reference to the contained value.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    #[inline]
    pub unsafe fn get_unchecked(&self) -> &T {
        &self.data
    }

    /// Returns a mutable reference to the contained value.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    #[inline]
    pub unsafe fn get_unchecked_mut(&mut self) -> &mut T {
        &mut self.data
    }

    /// Returns a pinned reference to the contained value.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    #[inline]
    pub unsafe fn get_unchecked_pinned(self: Pin<&Self>) -> Pin<&T> {
        // SAFETY: as long as `SendWrapper` is pinned, the inner data is pinned too.
        unsafe { self.map_unchecked(|s| &*s.data) }
    }

    /// Returns a pinned mutable reference to the contained value.
    ///
    /// # Safety
    ///
    /// The caller should be in the same thread as the creator.
    #[inline]
    pub unsafe fn get_unchecked_pinned_mut(self: Pin<&mut Self>) -> Pin<&mut T> {
        // SAFETY: as long as `SendWrapper` is pinned, the inner data is pinned too.
        unsafe { self.map_unchecked_mut(|s| &mut *s.data) }
    }

    /// Returns a reference to the contained value, if valid.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        if self.valid() { Some(&self.data) } else { None }
    }

    /// Returns a mutable reference to the contained value, if valid.
    #[inline]
    pub fn get_mut(&mut self) -> Option<&mut T> {
        if self.valid() {
            Some(&mut self.data)
        } else {
            None
        }
    }

    /// Returns a pinned reference to the contained value, if valid.
    #[inline]
    pub fn get_pinned(self: Pin<&Self>) -> Option<Pin<&T>> {
        if self.valid() {
            // SAFETY: the same thread as the creator
            Some(unsafe { self.get_unchecked_pinned() })
        } else {
            None
        }
    }

    /// Returns a pinned mutable reference to the contained value, if valid.
    #[inline]
    pub fn get_pinned_mut(self: Pin<&mut Self>) -> Option<Pin<&mut T>> {
        if self.valid() {
            // SAFETY: the same thread as the creator
            Some(unsafe { self.get_unchecked_pinned_mut() })
        } else {
            None
        }
    }

    /// Returns a tracker that can be used to check if the current thread is
    /// the same as the creator thread.
    #[inline]
    pub fn tracker(&self) -> SendWrapper<()> {
        SendWrapper {
            data: ManuallyDrop::new(()),
            thread_id: self.thread_id,
        }
    }
}

unsafe impl<T> Send for SendWrapper<T> {}
unsafe impl<T> Sync for SendWrapper<T> {}

impl<T> Drop for SendWrapper<T> {
    /// Drops the contained value.
    ///
    /// # Panics
    ///
    /// Dropping panics if it is done from a different thread than the one the
    /// `SendWrapper<T>` instance has been created with.
    ///
    /// Exceptions:
    /// - There is no extra panic if the thread is already panicking/unwinding.
    ///   This is because otherwise there would be double panics (usually
    ///   resulting in an abort) when dereferencing from a wrong thread.
    /// - If `T` has a trivial drop ([`needs_drop::<T>()`] is false) then this
    ///   method never panics.
    ///
    /// [`needs_drop::<T>()`]: std::mem::needs_drop
    #[track_caller]
    fn drop(&mut self) {
        // If the drop is trivial (`needs_drop` = false), then dropping `T` can't access
        // it and so it can be safely dropped on any thread.
        if !mem::needs_drop::<T>() || self.valid() {
            unsafe {
                // Drop the inner value
                //
                // SAFETY:
                // - We've just checked that it's valid to drop `T` on this thread
                // - We only move out from `self.data` here and in drop, so `self.data` is
                //   present
                ManuallyDrop::drop(&mut self.data);
            }
        } else {
            invalid_drop()
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for SendWrapper<T> {
    /// Formats the value using the given formatter.
    ///
    /// # Panics
    ///
    /// Formatting panics if it is done from a different thread than the one
    /// the `SendWrapper<T>` instance has been created with.
    #[track_caller]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("SendWrapper");
        if let Some(data) = self.get() {
            f.field("data", data);
        } else {
            f.field("data", &"<invalid>");
        }
        f.field("thread_id", &self.thread_id).finish()
    }
}

impl<T: Clone> Clone for SendWrapper<T> {
    /// Returns a copy of the value.
    ///
    /// # Panics
    ///
    /// Cloning panics if it is done from a different thread than the one
    /// the `SendWrapper<T>` instance has been created with.
    #[track_caller]
    fn clone(&self) -> Self {
        Self::new(self.get().unwrap_or_else(|| invalid_deref()).clone())
    }
}

#[cold]
#[inline(never)]
#[track_caller]
fn invalid_deref() -> ! {
    const DEREF_ERROR: &str = "Dereferenced SendWrapper<T> variable from a thread different to \
                               the one it has been created with.";

    panic!("{}", DEREF_ERROR)
}

#[cold]
#[inline(never)]
#[track_caller]
#[cfg(feature = "futures")]
fn invalid_poll() -> ! {
    const POLL_ERROR: &str = "Polling SendWrapper<T> variable from a thread different to the one \
                              it has been created with.";

    panic!("{}", POLL_ERROR)
}

#[cold]
#[inline(never)]
#[track_caller]
fn invalid_drop() {
    const DROP_ERROR: &str = "Dropped SendWrapper<T> variable from a thread different to the one \
                              it has been created with.";

    if !thread::panicking() {
        // panic because of dropping from wrong thread
        // only do this while not unwinding (could be caused by deref from wrong thread)
        panic!("{}", DROP_ERROR)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        rc::Rc,
        sync::{Arc, mpsc::channel},
        thread,
    };

    use super::SendWrapper;

    #[test]
    fn test_valid() {
        let (sender, receiver) = channel();
        let w = SendWrapper::new(Rc::new(42));
        assert!(w.valid());
        let t = thread::spawn(move || {
            // move SendWrapper back to main thread, so it can be dropped from there
            sender.send(w).unwrap();
        });
        let w2 = receiver.recv().unwrap();
        assert!(w2.valid());
        assert!(t.join().is_ok());
    }

    #[test]
    fn test_invalid() {
        let w = SendWrapper::new(Rc::new(42));
        let t = thread::spawn(move || {
            assert!(!w.valid());
        });
        let join_result = t.join();
        assert!(join_result.is_err());
    }

    #[test]
    fn test_drop_panic() {
        let w = SendWrapper::new(Rc::new(42));
        let t = thread::spawn(move || {
            drop(w);
        });
        let join_result = t.join();
        assert!(join_result.is_err());
    }

    #[test]
    fn test_take() {
        let w = SendWrapper::new(Rc::new(42));
        let inner: Rc<usize> = w.take().unwrap();
        assert_eq!(42, *inner);
    }

    #[test]
    fn test_take_panic() {
        let w = SendWrapper::new(Rc::new(42));
        let t = thread::spawn(move || {
            let _ = w.take().unwrap();
        });
        assert!(t.join().is_err());
    }
    #[test]
    fn test_sync() {
        // Arc<T> can only be sent to another thread if T Sync
        let arc = Arc::new(SendWrapper::new(42));
        thread::spawn(move || {
            let _ = arc;
        });
    }

    #[test]
    fn test_debug() {
        let w = SendWrapper::new(Rc::new(42));
        let info = format!("{:?}", w);
        assert!(info.contains("SendWrapper {"));
        assert!(info.contains("data: 42,"));
        assert!(info.contains("thread_id: ThreadId("));
    }

    #[test]
    fn test_debug_invalid() {
        let w = SendWrapper::new(Rc::new(42));
        let t = thread::spawn(move || {
            let info = format!("{:?}", w);
            assert!(info.contains("SendWrapper {"));
            assert!(info.contains("data: \"<invalid>\","));
            assert!(info.contains("thread_id: ThreadId("));
            w
        });
        assert!(t.join().is_ok());
    }

    #[test]
    fn test_clone() {
        let w1 = SendWrapper::new(Rc::new(42));
        let w2 = w1.clone();
        assert_eq!(format!("{:?}", w1), format!("{:?}", w2));
    }

    #[test]
    fn test_clone_panic() {
        let w = SendWrapper::new(Rc::new(42));
        let t = thread::spawn(move || {
            let _ = w.clone();
        });
        assert!(t.join().is_err());
    }
}
