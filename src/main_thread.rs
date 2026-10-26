use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

/// State that only the engine thread may touch, so that it can live in a
/// `static` without `static mut`.
///
/// GoldSrc calls every game DLL and metamod entry point from one thread. The
/// first thread that reads any `MainThreadCell` becomes the engine thread;
/// any other thread that reads one gets `None` from [`try_get`] or a panic
/// from [`get`]. The check is a thread-local read, so release builds keep it.
///
/// [`try_get`]: MainThreadCell::try_get
/// [`get`]: MainThreadCell::get
pub struct MainThreadCell<T>(T);

// SAFETY: the inner value is only reachable through `try_get`, which refuses
// every thread except the engine thread, so it is never shared across threads
// even when `T` is neither `Send` nor `Sync`.
unsafe impl<T> Sync for MainThreadCell<T> {}

static ENGINE_THREAD_CLAIMED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ON_ENGINE_THREAD: Cell<bool> = const { Cell::new(false) };
}

impl<T> MainThreadCell<T> {
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns the value, or `None` when called off the engine thread.
    pub fn try_get(&self) -> Option<&T> {
        Self::on_engine_thread().then_some(&self.0)
    }

    /// Returns the value.
    ///
    /// # Panics
    ///
    /// Panics when called off the engine thread.
    pub fn get(&self) -> &T {
        self.try_get()
            .expect("metamod state used off the engine thread")
    }

    fn on_engine_thread() -> bool {
        ON_ENGINE_THREAD.get()
            || (ENGINE_THREAD_CLAIMED
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
                && {
                    ON_ENGINE_THREAD.set(true);
                    true
                })
    }
}

#[cfg(test)]
mod tests {
    use super::MainThreadCell;

    static COUNTER: MainThreadCell<std::cell::Cell<u32>> =
        MainThreadCell::new(std::cell::Cell::new(0));

    #[test]
    fn first_thread_owns_the_cell_and_others_are_refused() {
        COUNTER.get().set(1);
        let other_thread = std::thread::spawn(|| COUNTER.try_get().is_none())
            .join()
            .expect("probe thread");
        assert!(other_thread, "a second thread must not reach the value");
        assert_eq!(COUNTER.get().get(), 1);
    }
}
