//! The C entry points that [`export_plugin!`](crate::export_plugin)
//! generates. Not public API: use `#[metamod_p::plugin]`.

use std::{
    cell::{RefCell, UnsafeCell},
    error::Error,
    ffi::{c_char, c_int},
    ptr::NonNull,
};

use crate::{
    ffi::{c_str, guard, FALSE, TRUE},
    host::Host,
    logger,
    plugin::{dll_hooks, engine_hooks, new_dll_hooks},
    sys::{
        enginefuncs_t, gamedll_funcs_t, globalvars_t, meta_globals_t, mutil_funcs_t, plugin_info_t,
        META_FUNCTIONS, PLUG_LOADTIME, PL_UNLOAD_REASON,
    },
    LoadTime, MainThreadCell, Plugin, UnloadReason,
};

/// `Meta_Query`'s version argument: `char *` in Metamod-P and
/// `const char *` in metamod-svencoop.
#[cfg(feature = "fallguys")]
pub type InterfaceVersion = *const c_char;
#[cfg(not(feature = "fallguys"))]
pub type InterfaceVersion = *mut c_char;

/// Links a [`Plugin`] type to the static that holds it.
pub trait Exported: Plugin {
    fn instance() -> &'static MainThreadCell<Instance<Self>>;
}

/// The exported plugin: its info for metamod and, while attached, its value.
pub struct Instance<P> {
    /// Metamod keeps a `*mut` to this, so it lives in writable memory.
    info:              UnsafeCell<plugin_info_t>,
    pub(crate) plugin: RefCell<Option<P>>,
}

impl<P: Plugin> Instance<P> {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            info:   UnsafeCell::new(P::INFO.to_raw()),
            plugin: RefCell::new(None),
        }
    }
}

/// Runs an entry point and turns its outcome into metamod's TRUE or FALSE.
fn status(entry_point: &str, body: impl FnOnce() -> Result<(), Box<dyn Error>>) -> c_int {
    match guard(entry_point, body) {
        Some(Ok(())) => TRUE,
        Some(Err(error)) => {
            log::error!("{entry_point}: {error}");
            FALSE
        }
        None => FALSE,
    }
}

/// # Safety
///
/// Pointers must be null or valid, as the engine passes them.
pub unsafe fn give_fnptrs_to_dll(
    engine_functions: *const enginefuncs_t,
    globals: *mut globalvars_t,
) {
    logger::install();
    guard("GiveFnptrsToDll", || {
        // SAFETY: the engine lends its function table for the call; it is
        // copied.
        match (unsafe { engine_functions.as_ref() }, NonNull::new(globals)) {
            (Some(functions), Some(globals)) => Host::get().receive_engine(*functions, globals),
            _ => log::error!("GiveFnptrsToDll: the engine passed no function table or globals"),
        }
    });
}

/// # Safety
///
/// Pointers must be null or valid, as metamod passes them.
pub unsafe fn meta_query<P: Exported>(
    interface_version: InterfaceVersion,
    plugin_info: *mut *mut plugin_info_t,
    util_functions: *const mutil_funcs_t,
) -> c_int {
    status("Meta_Query", || {
        let info = P::instance().get().info.get();
        // Hand out the info first, so metamod can name the plugin even if
        // the query fails.
        if let Some(slot) = NonNull::new(plugin_info) {
            // SAFETY: metamod passes a writable slot for the info pointer.
            unsafe { slot.write(info) }
        }
        // SAFETY: metamod lends its utility table for the call; it is copied.
        let util =
            unsafe { util_functions.as_ref() }.ok_or("metamod passed no utility functions")?;
        Host::get().connect(*util, info)?;
        // SAFETY: metamod passes its interface version as a C string.
        let interface =
            unsafe { c_str(interface_version) }.ok_or("metamod passed no interface version")?;
        P::query(interface)
    })
}

/// # Safety
///
/// Pointers must be null or valid, as metamod passes them.
pub unsafe fn meta_attach<P: Exported>(
    now: PLUG_LOADTIME,
    function_tables: *mut META_FUNCTIONS,
    meta_globals: *mut meta_globals_t,
    _game_functions: *mut gamedll_funcs_t,
) -> c_int {
    status("Meta_Attach", || {
        let now = LoadTime::from_raw(now).ok_or("metamod passed an unknown load time")?;
        // SAFETY: metamod lends its zeroed getter table for the call.
        let tables =
            unsafe { function_tables.as_mut() }.ok_or("metamod passed no function tables")?;
        let host = Host::get();
        host.meta_globals.set(Some(
            NonNull::new(meta_globals).ok_or("metamod passed no globals")?,
        ));
        let engine = host
            .engine
            .get()
            .ok_or("Meta_Query did not run before Meta_Attach")?;
        let instance = P::instance().get();
        if instance.plugin.borrow().is_some() {
            return Err("the plugin is already attached".into());
        }
        let plugin = P::attach(engine, now)?;
        *instance.plugin.borrow_mut() = Some(plugin);
        [tables.pfnGetEntityAPI2, tables.pfnGetEntityAPI2_Post] = dll_hooks::getters::<P>();
        [
            tables.pfnGetNewDLLFunctions,
            tables.pfnGetNewDLLFunctions_Post,
        ] = new_dll_hooks::getters::<P>();
        [
            tables.pfnGetEngineFunctions,
            tables.pfnGetEngineFunctions_Post,
        ] = engine_hooks::getters::<P>();
        Ok(())
    })
}

/// # Safety
///
/// Called by metamod; takes no pointers.
pub unsafe fn meta_detach<P: Exported>(now: PLUG_LOADTIME, reason: PL_UNLOAD_REASON) -> c_int {
    status("Meta_Detach", || {
        let now = LoadTime::from_raw(now).ok_or("metamod passed an unknown load time")?;
        // Take the plugin out before detaching it, so hooks that run meanwhile
        // see no plugin rather than a borrowed one.
        let plugin = P::instance().get().plugin.borrow_mut().take();
        if let Some(engine) = Host::engine() {
            engine.commands.borrow_mut().clear();
        }
        if let Some(plugin) = plugin {
            plugin.detach(now, UnloadReason::from_raw(reason));
        }
        Ok(())
    })
}

/// Exports a [`Plugin`] type through the C entry points that metamod looks
/// for: `GiveFnptrsToDll`, `Meta_Query`, `Meta_Attach` and `Meta_Detach`.
/// `#[metamod_p::plugin]` calls this; call it yourself only for an impl
/// without the attribute. One plugin per library.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        const _: () = {
            static INSTANCE: $crate::MainThreadCell<$crate::export::Instance<$plugin>> =
                $crate::MainThreadCell::new($crate::export::Instance::new());

            impl $crate::export::Exported for $plugin {
                fn instance() -> &'static $crate::MainThreadCell<$crate::export::Instance<Self>> {
                    &INSTANCE
                }
            }

            // The exports must match metamod's prototypes.
            let _: $crate::sys::META_QUERY_FN = Some(Meta_Query);
            let _: $crate::sys::META_ATTACH_FN = Some(Meta_Attach);
            let _: $crate::sys::META_DETACH_FN = Some(Meta_Detach);
        };

        /// Receives the engine functions (WINAPI on Windows).
        ///
        /// # Safety
        ///
        /// Called by metamod with null or valid pointers.
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn GiveFnptrsToDll(
            engine_functions: *mut $crate::sys::enginefuncs_t,
            globals: *mut $crate::sys::globalvars_t,
        ) {
            // SAFETY: forwarded from metamod.
            unsafe { $crate::export::give_fnptrs_to_dll(engine_functions, globals) }
        }

        /// Gives metamod the plugin info and receives its utility functions.
        ///
        /// # Safety
        ///
        /// Called by metamod with null or valid pointers.
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn Meta_Query(
            interface_version: $crate::export::InterfaceVersion,
            plugin_info: *mut *mut $crate::sys::plugin_info_t,
            util_functions: *mut $crate::sys::mutil_funcs_t,
        ) -> ::std::ffi::c_int {
            // SAFETY: forwarded from metamod.
            unsafe {
                $crate::export::meta_query::<$plugin>(
                    interface_version,
                    plugin_info,
                    util_functions,
                )
            }
        }

        /// Attaches the plugin and gives metamod its hook table getters.
        ///
        /// # Safety
        ///
        /// Called by metamod with null or valid pointers.
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn Meta_Attach(
            now: $crate::sys::PLUG_LOADTIME,
            function_tables: *mut $crate::sys::META_FUNCTIONS,
            meta_globals: *mut $crate::sys::meta_globals_t,
            game_functions: *mut $crate::sys::gamedll_funcs_t,
        ) -> ::std::ffi::c_int {
            // SAFETY: forwarded from metamod.
            unsafe {
                $crate::export::meta_attach::<$plugin>(
                    now,
                    function_tables,
                    meta_globals,
                    game_functions,
                )
            }
        }

        /// Detaches the plugin.
        ///
        /// # Safety
        ///
        /// Called by metamod.
        #[allow(non_snake_case)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn Meta_Detach(
            now: $crate::sys::PLUG_LOADTIME,
            reason: $crate::sys::PL_UNLOAD_REASON,
        ) -> ::std::ffi::c_int {
            // SAFETY: forwarded from metamod.
            unsafe { $crate::export::meta_detach::<$plugin>(now, reason) }
        }
    };
}
