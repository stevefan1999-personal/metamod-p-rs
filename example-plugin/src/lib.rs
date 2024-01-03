#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::ffi::{c_char, c_int, CStr};

use const_default::ConstDefault;
use log::{info, LevelFilter};
use logger::SimpleLogger;
use metamod_p::{
    edict_t, enginefuncs_t, gamedll_funcs_t, globalvars_t, meta_globals_t, mutil_funcs_t,
    plugin_info_t, qboolean, DLL_FUNCTIONS, ENGINE_INTERFACE_VERSION, INTERFACE_VERSION,
    META_FUNCTIONS, PLUG_LOADTIME, PL_UNLOAD_REASON,
};

use crate::utils::MetaResult;

pub(crate) static mut g_engfuncs: enginefuncs_t = enginefuncs_t::DEFAULT;
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
            // log::trace!("SetModel {e:?} {:?}", unsafe { CStr::from_ptr(m) });
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
            // log::trace!("CreateEntity");
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
    pfnClientConnect: {
        unsafe extern "C" fn ClientConnect(
            pEntity: *mut edict_t,
            pszName: *const ::std::ffi::c_char,
            pszAddress: *const ::std::ffi::c_char,
            szRejectReason: *mut [::std::ffi::c_char; 128usize],
        ) -> qboolean {
            log::trace!(
                "ClientConnect {pEntity:?} {:?} {:?} {:?}",
                CStr::from_ptr(pszName),
                CStr::from_ptr(pszAddress),
                CStr::from_ptr(szRejectReason.as_ref().unwrap().as_ptr())
            );
            MetaResult::Ignored.into()
        }

        Some(ClientConnect)
    },
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

pub(crate) mod globals;

#[no_mangle]
pub extern "system" fn GiveFnptrsToDll(
    pengfuncsFromEngine: *mut enginefuncs_t,
    pGlobals: *mut globalvars_t,
) {
    info!("hello world from GiveFnptrsToDll");
    unsafe {
        g_engfuncs = *pengfuncsFromEngine;
        globals::GLOBALS = pGlobals;
    }
}

#[no_mangle]
pub extern "C" fn Meta_Query(
    _ifvers: *mut ::std::os::raw::c_char,
    pPlugInfo: *mut *mut plugin_info_t,
    pMetaUtilFuncs: *mut mutil_funcs_t,
) -> c_int {
    unsafe {
        *pPlugInfo = &mut globals::PLUGIN_INFO;
        globals::META_UTIL_FUNCS = pMetaUtilFuncs;
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
        globals::META_GLOBALS = pMGlobals;
    }
    if pFunctionTable.is_null() {
        return false.into();
    }
    unsafe {
        *pFunctionTable = gMetaFunctionTable;
        globals::GAME_DLL_FUNCS = pGamedllFuncs;
    }
    true.into()
}

#[no_mangle]
pub extern "C" fn Meta_Detach(_now: PLUG_LOADTIME, _reason: PL_UNLOAD_REASON) -> c_int {
    info!("hello world from Meta_Detach");
    true.into()
}

pub mod logger;

pub mod utils;
