use std::ffi::{c_char, c_int, CStr};

use byte_strings::c_str;
use const_default::ConstDefault;
use log::{info, LevelFilter};
use logger::SimpleLogger;
use metamod_p::{
    edict_t, enginefuncs_t, gamedll_funcs_t, globalvars_t, meta_globals_t, mutil_funcs_t,
    plugin_info_t, DLL_FUNCTIONS, ENGINE_INTERFACE_VERSION, INTERFACE_VERSION,
    META_FUNCTIONS, META_INTERFACE_VERSION, PLUG_LOADTIME, PL_UNLOAD_REASON,
};

use crate::utils::MetaResult;

pub(crate) static mut g_engfuncs: enginefuncs_t = enginefuncs_t {
    ..enginefuncs_t::DEFAULT
};
pub(crate) static mut meta_engfuncs: enginefuncs_t = enginefuncs_t {
    pfnPrecacheSound: {
        extern "C" fn PrecacheSound(s: *mut c_char) -> c_int {
            log::trace!("sound={:?}", s);
            MetaResult::Ignored.into()
        }

        Some(PrecacheSound)
    },
    pfnSetModel: {
        extern "C" fn SetModel(e: *mut edict_t, m: *const c_char) {
            log::trace!("SetModel {e:?} {:?}", unsafe { CStr::from_ptr(m) });
            MetaResult::Ignored.into()
        }

        Some(SetModel)
    },
    pfnTime: {
        extern "C" fn Time() -> f32 {
            log::trace!("Time");
            MetaResult::Ignored.into()
        }

        Some(Time)
    },
    pfnCreateEntity: {
        extern "C" fn CreateEntity() -> *mut edict_t {
            log::trace!("CreateEntity");
            MetaResult::Ignored.into()
        }
        Some(CreateEntity)
    },
    pfnSetClientMaxspeed: {
        extern "C" fn SetClientMaxspeed(pEdict: *const edict_t, fNewMaxspeed: f32) {
            log::trace!("SetClientMaxspeed {pEdict:?} {fNewMaxspeed}");
            MetaResult::Ignored.into()
        }
        Some(SetClientMaxspeed)
    },
    ..enginefuncs_t::DEFAULT
};

pub(crate) static mut gFunctionTable: DLL_FUNCTIONS = DLL_FUNCTIONS {
    ..DLL_FUNCTIONS::DEFAULT
};
pub(crate) static mut gMetaFunctionTable: META_FUNCTIONS = META_FUNCTIONS {
    pfnGetEntityAPI2: {
        extern "C" fn GetEntityAPI2(
            pFunctionTable: *mut DLL_FUNCTIONS,
            interfaceVersion: *mut i32,
        ) -> c_int {
            info!("hello world from GetEntityAPI2");
            if pFunctionTable.is_null() {
                false
            } else if unsafe { *interfaceVersion } != INTERFACE_VERSION as i32 {
                unsafe { *interfaceVersion = INTERFACE_VERSION as i32 };
                false
            } else {
                unsafe { *pFunctionTable = gFunctionTable };
                true
            }
            .into()
        }

        Some(GetEntityAPI2)
    },
    pfnGetEngineFunctions: {
        extern "C" fn GetEngineFunctions(
            pengfuncsFromEngine: *mut enginefuncs_t,
            interfaceVersion: *mut i32,
        ) -> c_int {
            info!(
                "hello world from GetEngineFunctions {pengfuncsFromEngine:?} {interfaceVersion:?}"
            );
            if pengfuncsFromEngine.is_null() {
                false
            } else if unsafe { *interfaceVersion } != ENGINE_INTERFACE_VERSION as i32 {
                unsafe { *interfaceVersion = ENGINE_INTERFACE_VERSION as i32 };
                false
            } else {
                unsafe { *pengfuncsFromEngine = meta_engfuncs };
                true
            }
            .into()
        }

        Some(GetEngineFunctions)
    },
    ..META_FUNCTIONS::DEFAULT
};
pub(crate) static mut gpGlobals: *mut globalvars_t = std::ptr::null_mut();
pub(crate) static mut gpMetaGlobals: *mut meta_globals_t = std::ptr::null_mut();
pub(crate) static mut gpGamedllFuncs: *mut gamedll_funcs_t = std::ptr::null_mut();
pub(crate) static mut gpMetaUtilFuncs: *mut mutil_funcs_t = std::ptr::null_mut();

pub(crate) static mut Plugin_info: plugin_info_t = plugin_info_t {
    ifvers:     META_INTERFACE_VERSION.as_ptr() as _,
    name:       c_str!("minimal stub").as_ptr() as _,
    version:    c_str!("1.17").as_ptr() as _,
    date:       c_str!("2023/01/03").as_ptr() as _,
    author:     c_str!("Steve Fan <willday@metamod.org>").as_ptr() as _,
    url:        c_str!("http://www.metamod.org/").as_ptr() as _,
    logtag:     c_str!("STUB").as_ptr() as _,
    loadable:   PLUG_LOADTIME::PT_ANYTIME,
    unloadable: PLUG_LOADTIME::PT_ANYPAUSE,
};

#[no_mangle]
pub extern "system" fn GiveFnptrsToDll(
    pengfuncsFromEngine: *mut enginefuncs_t,
    pGlobals: *mut globalvars_t,
) {
    info!("hello world from GiveFnptrsToDll");
    unsafe {
        g_engfuncs = *pengfuncsFromEngine;
        gpGlobals = pGlobals;
    }
}

#[no_mangle]
pub extern "C" fn Meta_Query(
    _ifvers: *mut ::std::os::raw::c_char,
    pPlugInfo: *mut *mut plugin_info_t,
    pMetaUtilFuncs: *mut mutil_funcs_t,
) -> c_int {
    unsafe {
        *pPlugInfo = &mut Plugin_info;
        gpMetaUtilFuncs = pMetaUtilFuncs;
    }
    log::set_logger(&SimpleLogger)
        .map(|()| log::set_max_level(LevelFilter::Trace))
        .unwrap();

    true.into()
}

#[no_mangle]
pub extern "C" fn Meta_Attach(
    _now: PLUG_LOADTIME,
    pFunctionTable: *mut META_FUNCTIONS,
    pMGlobals: *mut meta_globals_t,
    pGamedllFuncs: *mut gamedll_funcs_t,
) -> c_int {
    info!("hello world from Meta_Attach");
    if pMGlobals.is_null() {
        return false.into();
    }
    unsafe {
        gpMetaGlobals = pMGlobals;
    }
    if pFunctionTable.is_null() {
        return false.into();
    }
    unsafe {
        *pFunctionTable = gMetaFunctionTable;
        gpGamedllFuncs = pGamedllFuncs;
    }
    true.into()
}

#[no_mangle]
pub extern "C" fn Meta_Detach(_now: PLUG_LOADTIME, _reason: PL_UNLOAD_REASON) -> c_int {
    info!("hello world from Meta_Detach");
    true.into()
}

pub mod logger;

pub mod utils {
    use metamod_p::META_RES;
    use std::mem::MaybeUninit;
    use crate::gpMetaGlobals;

    pub enum MetaResult<T> {
        Ignored,
        Handled,
        Override(T),
        Supercede(T),
    }
    
    impl<T> MetaResult<T> {
        pub unsafe fn original<'a>() -> Option<&'a T> {
            ((*gpMetaGlobals).orig_ret as *mut T).as_ref()
        }
    }
    
    impl<T> MetaResult<T> {
        pub fn into(self) -> T {
            let globals = unsafe { gpMetaGlobals.as_mut() };
    
            match self {
                MetaResult::Ignored => {
                    if let Some(globals) = globals {
                        globals.mres = META_RES::MRES_IGNORED;
                    }
                    unsafe { MaybeUninit::zeroed().assume_init() }
                }
                MetaResult::Handled => {
                    if let Some(globals) = globals {
                        globals.mres = META_RES::MRES_HANDLED;
                    }
                    unsafe { MaybeUninit::zeroed().assume_init() }
                }
                MetaResult::Override(x) => {
                    if let Some(globals) = globals {
                        globals.mres = META_RES::MRES_OVERRIDE;
                    }
                    x
                }
                MetaResult::Supercede(x) => {
                    if let Some(globals) = globals {
                        globals.mres = META_RES::MRES_SUPERCEDE;
                    }
                    x
                }
            }
        }
    }
    
}