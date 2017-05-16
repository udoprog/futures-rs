//! Executors
//!
//! This module contains tools for managing the raw execution of futures,
//! which is needed when building *executors* (places where futures can run).
//!
//! More information about executors can be [found online at tokio.rs][online].
//!
//! [online]: https://tokio.rs/docs/going-deeper-futures/tasks/

#[allow(deprecated)]
pub use task_impl::{Spawn, spawn, Unpark, Notify, Executor, Run};

pub use task_impl::{UnsafeNotify, NotifyHandle};

use std::fmt;
use std::marker;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{Ordering, AtomicBool};
use std::thread;

/// A trait implemented by *executors* which can execute futures to completion,
/// blocking the current thread until the future is done.
///
/// This trait is used by the `Future::wait` method and various other
/// `wait`-related methods on `Stream` and `Sink`. Runtimes can leverage this
/// trait to inject their own behavior for how to block waiting for a future
/// to complete. For example a simple application may not use this at all,
/// simply blocking the thread via standard `std::thread` mechanisms. An
/// application using an event loop may install the event loop as a local
/// executor so calls to `Future::wait` will simply turn the event loop while
/// blocking.
///
/// Note that this trait is a relatively low level detail that you likely won't
/// have to interact much with. It's recommended to consult your local
/// runtime's documentation to see if it's necessary to install it as an
/// executor. Crates such as `tokio-core`, for example, have a method to
/// install the `Core` as a local executor for the duration of a closure.
/// Crates like `rayon`, however, will automatically take care of this
/// trait.
pub trait Executor2 {
    /// Extract's a `NotifyHandle` from this executor, along with an id.
    ///
    /// This handle and id are then used to poll a future, sink, or stream,
    /// depending on the context. This method is called from the `wait` related
    /// methods of this crate whenever an object is about to be polled.
    ///
    /// If the object is not ready then the `NotifyHandle` specified will
    /// receive notifications with the id specified when the executor should be
    /// unblocked. After the object is not ready this crate will execute the
    /// `block` method below to block the current thread, and it's expected
    /// that the thread will wake up when the handle given here is notified.
    ///
    /// Typically this handle is stored inside of the executor itself, and
    /// more often than not the id is simply 0 or not actually used in the
    /// `Notify` implementation.
    fn notify(&self) -> (&NotifyHandle, u64);

    /// Blocks execution of the current thread until this executor's
    /// `NotifyHandle` is notified.
    ///
    /// This crate implements the various `wait` methods it provides with this
    /// function. When an object is polled and determined to be not ready this
    /// function is invoked to block the current thread. This may literally
    /// block the thread immediately, or it may do other "useful work" in the
    /// meantime, such as running an event loop.
    ///
    /// The crucial detail of this method is that it does not return until
    /// the object being polled is ready to get polled again. That is defined
    /// as when the `NotifyHandle` returned through the `notify` method above
    /// is notified. Once a notification is received implementors of this trait
    /// should ensure that this method returns shortly thereafter.
    ///
    /// # Panics
    ///
    /// It is expected that this method will not panic. This method may be
    /// called recursively due to multiple invocations of `Future::wait` on the
    /// stack.  In other words its expected for executors to handle
    /// waits-in-waits correctly.
    fn block(&self);
}

/// A simple "current thread" executor.
///
/// This executor implements the `Executor2` trait in this crate to block the
/// current thread via `std::thread::park` when the `block` method is invoked.
/// This is also the default executor for `Future::wait`, blocking the current
/// thread until a future is resolved.
///
/// You'll likely not need to use this type much, but it can serve as a
/// good example of how to implement an executor!
pub struct ThreadExecutor {
    unpark: Arc<ThreadUnpark>,
    handle: NotifyHandle,
    _not_send: marker::PhantomData<Rc<()>>,
}

struct ThreadUnpark {
    thread: thread::Thread,
    ready: AtomicBool,
}

impl ThreadExecutor {
    /// Acquires an executor for the current thread.
    pub fn current() -> ThreadExecutor {
        let unpark = Arc::new(ThreadUnpark {
            thread: thread::current(),
            ready: AtomicBool::new(false),
        });
        ThreadExecutor {
            unpark: unpark.clone(),
            handle: unpark.into(),
            _not_send: marker::PhantomData,
        }
    }
}

impl Executor2 for ThreadExecutor {
    fn notify(&self) -> (&NotifyHandle, u64) {
        (&self.handle, 0)
    }

    fn block(&self) {
        if !self.unpark.ready.swap(false, Ordering::SeqCst) {
            thread::park();
        }
    }
}

impl Notify for ThreadUnpark {
    fn notify(&self, _id: u64) {
        self.ready.store(true, Ordering::SeqCst);
        self.thread.unpark()
    }
}

impl fmt::Debug for ThreadExecutor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("ThreadExecutor")
         .field("thread", &self.unpark.thread)
         .finish()
    }
}

/// Installs an instance of `Executor2` as the local executor for the duration
/// of the closure, `f`.
///
/// This function will update this local thread's executor for the duration of
/// the closure specified. While the closure is being called all invocations
/// of `Future::wait` or other `wait` related functions will get routed to the
/// `ex` argument here.
///
/// If `with_executor` has been previously called then the `ex` specified here
/// will override the previously specified one. The previous executor will
/// again take over once this function returns.
///
/// This typically doesn't need to be called that often in your application,
/// nor does it typically need to be called directly. Runtimes such as
/// `tokio-core` and `rayon` will normally provide a method that invokes this
/// or simply take care of it for you.
///
/// # Panics
///
/// This function does not panic itself, but if the closure `f` panics then the
/// panic will be propagated outwards towards the caller.
pub fn with_executor<F, R>(ex: &Executor2, f: F) -> R
    where F: FnOnce() -> R,
{
    ::task_impl::set_executor(ex, f)
}
