use std::{
    ffi::{c_char, c_int, CStr},
    ptr::NonNull,
};

use crate::{
    export::Exported,
    ffi::{c_str, guard, FALSE, TRUE},
    host::Host,
    sys::{edict_t, META_RES},
    Edict, Engine, RejectReason, Returned,
};

/// Maps a hook's Rust return type to the C type metamod expects.
pub(crate) trait HookReturn: Sized {
    type Raw: Copy;
    /// What metamod gets when the hook returns no value. Metamod ignores it
    /// then, but C still needs a value of the right type.
    const PLACEHOLDER: Self::Raw;
    fn into_raw(self) -> Self::Raw;
    fn from_raw(raw: Self::Raw) -> Self;
}

impl HookReturn for () {
    type Raw = ();

    const PLACEHOLDER: Self::Raw = ();

    fn into_raw(self) {}

    fn from_raw((): ()) {}
}

impl HookReturn for bool {
    type Raw = c_int;

    const PLACEHOLDER: Self::Raw = FALSE;

    fn into_raw(self) -> c_int {
        self.into()
    }

    fn from_raw(raw: c_int) -> Self {
        raw != FALSE
    }
}

impl HookReturn for i32 {
    type Raw = c_int;

    const PLACEHOLDER: Self::Raw = 0;

    fn into_raw(self) -> c_int {
        self
    }

    fn from_raw(raw: c_int) -> Self {
        raw
    }
}

/// Converts a raw hook argument to the type the hook method takes. `None`
/// means the safe signature cannot represent it, such as a null entity.
pub(crate) trait FromRaw<Raw>: Sized {
    /// # Safety
    ///
    /// Pointers in `raw` must be valid for the hook call, as metamod
    /// guarantees for the arguments it passes.
    unsafe fn from_raw(raw: Raw, engine: &'static Engine) -> Option<Self>;
}

impl<T> FromRaw<T> for T {
    unsafe fn from_raw(raw: T, _: &'static Engine) -> Option<Self> {
        Some(raw)
    }
}

impl FromRaw<*mut edict_t> for Edict {
    unsafe fn from_raw(raw: *mut edict_t, engine: &'static Engine) -> Option<Self> {
        // SAFETY: hook entity arguments are null or engine edicts.
        unsafe { Edict::from_raw(raw, engine) }
    }
}

impl FromRaw<*const edict_t> for Edict {
    unsafe fn from_raw(raw: *const edict_t, engine: &'static Engine) -> Option<Self> {
        // SAFETY: hook entity arguments are null or engine edicts.
        unsafe { Edict::from_raw(raw.cast_mut(), engine) }
    }
}

impl FromRaw<*mut edict_t> for Option<Edict> {
    unsafe fn from_raw(raw: *mut edict_t, engine: &'static Engine) -> Option<Self> {
        // SAFETY: hook entity arguments are null or engine edicts.
        Some(unsafe { Edict::from_raw(raw, engine) })
    }
}

impl FromRaw<*const c_char> for &CStr {
    unsafe fn from_raw(raw: *const c_char, _: &'static Engine) -> Option<Self> {
        // SAFETY: hook string arguments live for the call.
        unsafe { c_str(raw) }
    }
}

impl FromRaw<*mut c_char> for &CStr {
    unsafe fn from_raw(raw: *mut c_char, _: &'static Engine) -> Option<Self> {
        // SAFETY: hook string arguments live for the call.
        unsafe { c_str(raw) }
    }
}

impl FromRaw<*mut [c_char; RejectReason::CAPACITY]> for &mut RejectReason {
    unsafe fn from_raw(
        raw: *mut [c_char; RejectReason::CAPACITY],
        _: &'static Engine,
    ) -> Option<Self> {
        // SAFETY: the engine lends its reject reason buffer for the call and
        // `RejectReason` is a transparent wrapper around it.
        NonNull::new(raw).map(|buffer| unsafe { &mut *buffer.as_ptr().cast::<RejectReason>() })
    }
}

impl FromRaw<*const f32> for Option<[f32; 3]> {
    unsafe fn from_raw(raw: *const f32, _: &'static Engine) -> Option<Self> {
        // SAFETY: a non-null origin points to a `vec3_t` for the call.
        Some(NonNull::new(raw.cast_mut()).map(|origin| unsafe { origin.cast::<[f32; 3]>().read() }))
    }
}

/// Runs a hook method of the attached plugin and reports its result, a
/// `META_RES` code and an optional value, to metamod. Every outcome sets
/// `mres`, so metamod never warns about an unset result: a missing plugin,
/// an unrepresentable argument or a panic all count as `MRES_IGNORED`.
pub(crate) fn dispatch<P: Exported, R: HookReturn>(
    hook: &'static str,
    call: impl FnOnce(&P, &'static Engine) -> Option<(META_RES, Option<R>)>,
) -> R::Raw {
    let result = guard(hook, || {
        let engine = Host::engine()?;
        let _call = Host::get().enter();
        let plugin = P::instance().get().plugin.borrow();
        let result = call(plugin.as_ref()?, engine);
        if result.is_none() {
            log::warn!("{hook}: skipped a call that the hook cannot represent");
        }
        result
    })
    .flatten();
    let (code, value) = result.unwrap_or((META_RES::MRES_IGNORED, None));
    Host::set_result(code);
    value.map_or(R::PLACEHOLDER, R::into_raw)
}

/// [`dispatch`] for a post hook that reads the call's return values.
pub(crate) fn dispatch_post<P: Exported, R: HookReturn>(
    hook: &'static str,
    call: impl FnOnce(&P, &'static Engine, Returned<R>) -> Option<(META_RES, Option<R>)>,
) -> R::Raw {
    dispatch::<P, R>(hook, |plugin, engine| {
        call(plugin, engine, Host::returned::<R>()?)
    })
}

/// Fills a function table that metamod asks for, after checking that both
/// sides were built for the same interface version.
///
/// `install` writes single entries through the pointer and never makes a
/// reference to the whole table: metamod allocates the table with its own
/// headers, and its engine table (632 bytes on i686 in metamod-svencoop) is
/// shorter than the Metamod-P `enginefuncs_t` (636 bytes).
///
/// # Safety
///
/// `table` and `version` must be null or valid for writes, as metamod
/// passes them, and every entry that `install` writes must be in both
/// header sets.
pub(crate) unsafe fn give_table<T>(
    table: *mut T,
    version: *mut c_int,
    expected: c_int,
    install: unsafe fn(NonNull<T>),
) -> c_int {
    guard("function table request", || {
        // SAFETY: metamod passes its own version variable.
        let (Some(table), Some(version)) = (NonNull::new(table), unsafe { version.as_mut() })
        else {
            log::error!("metamod asked for a function table without passing one");
            return FALSE;
        };
        if *version != expected {
            log::error!("metamod expects interface version {version}, plugin has {expected}");
            *version = expected;
            return FALSE;
        }
        // SAFETY: metamod's zeroed table holds every entry of the hook
        // list, as the caller guarantees.
        unsafe { install(table) };
        TRUE
    })
    .unwrap_or(FALSE)
}

/// Defines the `Plugin` trait with a pre and a post method per hook. For each
/// function table it also generates the C trampolines that call those
/// methods, the installer that only fills in the hooks a plugin overrides,
/// and the getter that metamod calls for the table.
macro_rules! plugin_trait {
    (@result) => { () };
    (@result $ret:ty) => { $ret };

    (@post $P:ident $field:ident $hook_post:ident [$($arg:ident: $arg_ty:ty),*] []) => {
        dispatch::<$P, ()>(stringify!($field), move |plugin, engine| {
            $(
                // SAFETY: metamod passes arguments that are valid for the call.
                let $arg: $arg_ty = unsafe { FromRaw::from_raw($arg, engine) }?;
            )*
            Some(plugin.$hook_post($($arg),*).into_parts())
        })
    };
    (@post $P:ident $field:ident $hook_post:ident [$($arg:ident: $arg_ty:ty),*] [$ret:ty]) => {
        dispatch_post::<$P, $ret>(stringify!($field), move |plugin, engine, returned| {
            $(
                // SAFETY: metamod passes arguments that are valid for the call.
                let $arg: $arg_ty = unsafe { FromRaw::from_raw($arg, engine) }?;
            )*
            Some(plugin.$hook_post($($arg,)* returned).into_parts())
        })
    };

    (
        $(#[$trait_attribute:meta])*
        pub trait Plugin { $($lifecycle:tt)* }
        $(
            $(#[$table_doc:meta])*
            mod $table:ident($Table:ident, $version:expr) {
                $(
                    $(#[$hook_doc:meta])*
                    fn $hook:ident / $hook_post:ident($($arg:ident: $arg_ty:ty),* $(,)?)
                        $(-> $ret:ty)? => $field:ident($($raw:ident: $raw_ty:ty),* $(,)?);
                )*
            }
        )*
    ) => {
        $(#[$trait_attribute])*
        pub trait Plugin: Sized + 'static {
            $($lifecycle)*

            $($(
                $(#[$hook_doc])*
                // Hook signatures mirror the C functions they intercept.
                #[allow(unused_variables, clippy::too_many_arguments)]
                fn $hook(&self, $($arg: $arg_ty),*) -> MetaResult<plugin_trait!(@result $($ret)?)> {
                    MetaResult::Ignored
                }

                #[doc = concat!(
                    "Post hook of [`Plugin::", stringify!($hook), "`]: runs after the real function."
                )]
                #[allow(unused_variables, clippy::too_many_arguments)]
                fn $hook_post(
                    &self,
                    $($arg: $arg_ty,)*
                    $(returned: Returned<$ret>,)?
                ) -> PostResult<plugin_trait!(@result $($ret)?)> {
                    PostResult::Ignored
                }
            )*)*
        }

        $(
            $(#[$table_doc])*
            pub(crate) mod $table {
                use super::*;

                $(
                    // Hooks without arguments never need the engine handle.
                    #[allow(unused_variables)]
                    unsafe extern "C" fn $hook<P: Exported>($($raw: $raw_ty),*)
                        $(-> <$ret as HookReturn>::Raw)?
                    {
                        dispatch::<P, plugin_trait!(@result $($ret)?)>(
                            stringify!($field),
                            move |plugin, engine| {
                                $(
                                    // SAFETY: metamod passes arguments that are valid for the call.
                                    let $arg: $arg_ty = unsafe { FromRaw::from_raw($arg, engine) }?;
                                )*
                                Some(plugin.$hook($($arg),*).into_parts())
                            },
                        )
                    }

                    #[allow(unused_variables)]
                    unsafe extern "C" fn $hook_post<P: Exported>($($raw: $raw_ty),*)
                        $(-> <$ret as HookReturn>::Raw)?
                    {
                        plugin_trait!(@post P $field $hook_post [$($arg: $arg_ty),*] [$($ret)?])
                    }
                )*

                /// # Safety
                ///
                /// Every entry of this hook list must be writable in `table`.
                unsafe fn install_pre<P: Exported>(table: ::std::ptr::NonNull<$Table>) {
                    $(
                        if P::OVERRIDES.contains(stringify!($hook)) {
                            // SAFETY: the caller guarantees that the entry is writable.
                            unsafe {
                                ::std::ptr::addr_of_mut!((*table.as_ptr()).$field).write(Some($hook::<P>))
                            }
                        }
                    )*
                }

                /// # Safety
                ///
                /// Every entry of this hook list must be writable in `table`.
                unsafe fn install_post<P: Exported>(table: ::std::ptr::NonNull<$Table>) {
                    $(
                        if P::OVERRIDES.contains(stringify!($hook_post)) {
                            // SAFETY: the caller guarantees that the entry is writable.
                            unsafe {
                                ::std::ptr::addr_of_mut!((*table.as_ptr()).$field).write(Some($hook_post::<P>))
                            }
                        }
                    )*
                }

                unsafe extern "C" fn get_pre<P: Exported>(table: *mut $Table, version: *mut c_int) -> c_int {
                    // SAFETY: metamod passes null or valid pointers, and the hook
                    // list only has entries that both header sets share.
                    unsafe { give_table(table, version, $version as c_int, install_pre::<P>) }
                }

                unsafe extern "C" fn get_post<P: Exported>(table: *mut $Table, version: *mut c_int) -> c_int {
                    // SAFETY: as in `get_pre`.
                    unsafe { give_table(table, version, $version as c_int, install_post::<P>) }
                }

                /// The pre and post getters for `META_FUNCTIONS`. A getter is
                /// only given when the plugin overrides a hook of that kind,
                /// so metamod never calls this plugin for anything else.
                pub(crate) fn getters<P: Exported>()
                    -> [Option<unsafe extern "C" fn(*mut $Table, *mut c_int) -> c_int>; 2]
                {
                    let overrides = |hooks: &[&str]| hooks.iter().any(|hook| P::OVERRIDES.contains(hook));
                    [
                        overrides(&[$(stringify!($hook)),*]).then_some(get_pre::<P> as _),
                        overrides(&[$(stringify!($hook_post)),*]).then_some(get_post::<P> as _),
                    ]
                }
            }
        )*
    };
}
pub(crate) use plugin_trait;
