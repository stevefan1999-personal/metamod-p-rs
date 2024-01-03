use byte_strings::c_str;
use metamod_p::{
    gamedll_funcs_t, globalvars_t, meta_globals_t, mutil_funcs_t, plugin_info_t,
    META_INTERFACE_VERSION, PLUG_LOADTIME,
};

pub(crate) static mut GLOBALS: *mut globalvars_t = std::ptr::null_mut();
pub(crate) static mut META_GLOBALS: *mut meta_globals_t = std::ptr::null_mut();
pub(crate) static mut GAME_DLL_FUNCS: *mut gamedll_funcs_t = std::ptr::null_mut();
pub(crate) static mut META_UTIL_FUNCS: *mut mutil_funcs_t = std::ptr::null_mut();
pub(crate) static mut PLUGIN_INFO: plugin_info_t = plugin_info_t {
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
