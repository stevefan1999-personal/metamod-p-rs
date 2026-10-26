use std::{
    any::Any,
    ffi::{c_char, c_int, CStr, CString},
    mem,
    panic::{self, AssertUnwindSafe},
};

/// C truth values, as metamod entry points return them.
pub(crate) const TRUE: c_int = 1;
pub(crate) const FALSE: c_int = 0;

/// Runs `body`, turning a panic into `None` plus an error log, because an
/// unwind that reaches C aborts the whole server.
pub(crate) fn guard<T>(context: &str, body: impl FnOnce() -> T) -> Option<T> {
    panic::catch_unwind(AssertUnwindSafe(body))
        .map_err(|payload| {
            log::error!("{context} panicked: {}", panic_message(payload.as_ref()));
            // The payload's Drop is plugin code too: a second panic from it
            // is caught, and its own payload is leaked rather than dropped.
            if let Err(nested) = panic::catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                mem::forget(nested);
            }
        })
        .ok()
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// Borrows a C string, or `None` for a null pointer.
///
/// # Safety
///
/// A non-null `ptr` must point to a NUL-terminated string that stays alive
/// and unchanged for `'a`.
pub(crate) unsafe fn c_str<'a>(ptr: *const c_char) -> Option<&'a CStr> {
    // SAFETY: the caller upholds the contract above for non-null pointers.
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) })
}

/// Copies `text` into a C string. Interior NULs are dropped: C would stop
/// reading at the first one anyway.
pub(crate) fn to_c_string(text: &str) -> CString {
    CString::new(text).unwrap_or_else(|error| {
        let mut bytes = error.into_vec();
        bytes.retain(|&byte| byte != 0);
        CString::new(bytes).expect("interior NULs were removed")
    })
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    #[test]
    fn null_c_string_is_absent() {
        assert_eq!(unsafe { c_str(ptr::null()) }, None);
    }

    #[test]
    fn c_string_borrows_the_text() {
        assert_eq!(unsafe { c_str(c"hl_c01_a1".as_ptr()) }, Some(c"hl_c01_a1"));
    }

    #[test]
    fn interior_nul_is_dropped_instead_of_truncating() {
        assert_eq!(to_c_string("rs\0_hello").as_c_str(), c"rs_hello");
    }

    #[test]
    fn guard_contains_a_panic() {
        assert_eq!(guard("test", || -> u8 { panic!("boom") }), None);
        assert_eq!(guard("test", || 7), Some(7));
    }

    /// A value whose Drop panics, as plugin code may.
    struct PanicsOnDrop;

    impl Drop for PanicsOnDrop {
        fn drop(&mut self) {
            panic!("drop failed");
        }
    }

    #[test]
    fn guard_contains_a_payload_whose_drop_panics() {
        assert_eq!(
            guard("test", || -> u8 { panic::panic_any(PanicsOnDrop) }),
            None
        );
    }
}
