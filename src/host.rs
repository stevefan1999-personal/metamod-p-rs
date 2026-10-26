use std::cell::OnceCell;

use crate::{Engine, MainThreadCell};

/// Plugin-wide state that metamod hands over through the C entry points.
pub(crate) struct Host {
    pub(crate) engine: OnceCell<Engine>,
}

static HOST: MainThreadCell<Host> = MainThreadCell::new(Host {
    engine: OnceCell::new(),
});

impl Host {
    /// The engine handle once `Meta_Query` has built it; `None` before that
    /// or off the engine thread.
    pub(crate) fn engine() -> Option<&'static Engine> {
        HOST.try_get()?.engine.get()
    }
}
