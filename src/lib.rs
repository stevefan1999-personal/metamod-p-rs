#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::mem::MaybeUninit;

use crate::sdk::globalvars_t;
use crate::sdk::enginefuncs_t;

pub mod sdk;

pub static mut g_engfuncs: enginefuncs_t = unsafe { MaybeUninit::zeroed().assume_init() };
pub static mut gpGlobals: *mut globalvars_t = std::ptr::null_mut();

#[no_mangle]
pub extern "system" fn GiveFnptrsToDll(pengfuncsFromEngine: *mut enginefuncs_t, pGlobals: *mut globalvars_t) {
    unsafe {
        g_engfuncs = *pengfuncsFromEngine;
        gpGlobals = pGlobals;
    }
}   