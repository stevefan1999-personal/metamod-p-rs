use std::{
    cell::{Cell, OnceCell},
    ptr::{addr_of, addr_of_mut, NonNull},
};

use crate::{
    hooks::HookReturn,
    sys::{enginefuncs_t, globalvars_t, meta_globals_t, mutil_funcs_t, plid_t, META_RES},
    Engine, MainThreadCell, Returned,
};

/// Plugin-wide state that metamod hands over through the C entry points.
pub(crate) struct Host {
    /// What `GiveFnptrsToDll` received, until `Meta_Query` adds the rest.
    engine_tables:           Cell<Option<(enginefuncs_t, NonNull<globalvars_t>)>>,
    pub(crate) engine:       OnceCell<Engine>,
    pub(crate) meta_globals: Cell<Option<NonNull<meta_globals_t>>>,
}

static HOST: MainThreadCell<Host> = MainThreadCell::new(Host {
    engine_tables: Cell::new(None),
    engine:        OnceCell::new(),
    meta_globals:  Cell::new(None),
});

impl Host {
    /// # Panics
    ///
    /// Panics off the engine thread.
    pub(crate) fn get() -> &'static Self {
        HOST.get()
    }

    /// The engine handle once `Meta_Query` has built it; `None` before that
    /// or off the engine thread.
    pub(crate) fn engine() -> Option<&'static Engine> {
        HOST.try_get()?.engine.get()
    }

    pub(crate) fn receive_engine(&self, functions: enginefuncs_t, globals: NonNull<globalvars_t>) {
        self.engine_tables.set(Some((functions, globals)));
    }

    /// Builds the engine handle from the tables of `GiveFnptrsToDll` and
    /// `Meta_Query`.
    pub(crate) fn connect(
        &self,
        util: mutil_funcs_t,
        plugin_id: plid_t,
    ) -> Result<&Engine, &'static str> {
        let (functions, globals) = self
            .engine_tables
            .get()
            .ok_or("GiveFnptrsToDll did not run before Meta_Query")?;
        // SAFETY: the tables come from the engine and metamod, and the plugin
        // id points to the plugin's static info.
        Ok(self
            .engine
            .get_or_init(|| unsafe { Engine::from_raw(functions, globals, util, plugin_id) }))
    }

    /// Tells metamod what the running hook did.
    pub(crate) fn set_result(code: META_RES) {
        if let Some(globals) = HOST.try_get().and_then(|host| host.meta_globals.get()) {
            // SAFETY: metamod's globals outlive the plugin.
            unsafe { addr_of_mut!((*globals.as_ptr()).mres).write(code) }
        }
    }

    /// The return values of the call that a post hook runs after.
    pub(crate) fn returned<R: HookReturn>() -> Option<Returned<R>> {
        let globals = Self::get().meta_globals.get()?.as_ptr();
        // SAFETY: during a post hook metamod points `orig_ret` at the call's
        // result and, only while the status is MRES_OVERRIDE, `override_ret`
        // at the override; both hold the hook's C return type.
        unsafe {
            let original = NonNull::new(addr_of!((*globals).orig_ret).read().cast::<R::Raw>())?;
            let overridden = (addr_of!((*globals).status).read() == META_RES::MRES_OVERRIDE)
                .then(|| NonNull::new(addr_of!((*globals).override_ret).read().cast::<R::Raw>()))
                .flatten();
            Some(Returned {
                original:   R::from_raw(original.read()),
                overridden: overridden.map(|value| R::from_raw(value.read())),
            })
        }
    }
}
