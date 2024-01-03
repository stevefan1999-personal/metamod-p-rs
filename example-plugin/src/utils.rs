use std::mem::MaybeUninit;

use metamod_p::META_RES;

pub enum MetaResult<T> {
    Ignored,
    Handled,
    Override(T),
    Supercede(T),
}

impl<T> MetaResult<T> {
    pub unsafe fn original<'a>() -> Option<&'a T> {
        ((*crate::globals::META_GLOBALS).orig_ret as *mut T).as_ref()
    }
}

impl<T> MetaResult<T> {
    pub fn into(self) -> T {
        let globals = unsafe { crate::globals::META_GLOBALS.as_mut() };

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
