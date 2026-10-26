use crate::sys::META_RES;

/// What a pre hook tells metamod about the call it intercepted.
///
/// Metamod keeps a pre hook's value only for `Supercede`. A pre hook that
/// returned `MRES_OVERRIDE` would lose its value: the caller would get
/// metamod's default for the function, such as `TRUE` for `ClientConnect`.
/// So there is no `Override` here; post hooks return [`PostResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaResult<T> {
    /// The hook did nothing; the real function runs.
    Ignored,
    /// The hook did something; the real function still runs.
    Handled,
    /// The real function is skipped and the caller gets this value.
    Supercede(T),
}

/// What a post hook tells metamod about the call that ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostResult<T> {
    /// The hook did nothing.
    Ignored,
    /// The hook did something; the caller still gets the call's value.
    Handled,
    /// The caller gets this value instead of the call's value. If a pre
    /// hook superceded the call, metamod ignores this value and returns the
    /// pre hook's value.
    Override(T),
}

impl<T> MetaResult<T> {
    /// The `META_RES` code for metamod and the value to return, if any.
    pub(crate) fn into_parts(self) -> (META_RES, Option<T>) {
        match self {
            Self::Ignored => (META_RES::MRES_IGNORED, None),
            Self::Handled => (META_RES::MRES_HANDLED, None),
            Self::Supercede(value) => (META_RES::MRES_SUPERCEDE, Some(value)),
        }
    }
}

impl<T> PostResult<T> {
    /// The `META_RES` code for metamod and the value to return, if any.
    pub(crate) fn into_parts(self) -> (META_RES, Option<T>) {
        match self {
            Self::Ignored => (META_RES::MRES_IGNORED, None),
            Self::Handled => (META_RES::MRES_HANDLED, None),
            Self::Override(value) => (META_RES::MRES_OVERRIDE, Some(value)),
        }
    }
}

/// Return values that a post hook can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Returned<T> {
    /// The value the call produced: the real function's result, or the
    /// value of a pre hook that superceded it.
    pub original:   T,
    /// `Some` when an earlier hook overrode the call: metamod then returns
    /// this value instead of `original`, unless a later post hook overrides
    /// it again. It is the value of the last post hook that returned
    /// [`PostResult::Override`], or metamod's default for the function (such
    /// as `true` for `ClientConnect`) when only a pre hook of another
    /// plugin returned `MRES_OVERRIDE`.
    pub overridden: Option<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_hook_results_map_to_metamod_codes() {
        assert_eq!(
            MetaResult::<bool>::Ignored.into_parts(),
            (META_RES::MRES_IGNORED, None)
        );
        assert_eq!(
            MetaResult::<bool>::Handled.into_parts(),
            (META_RES::MRES_HANDLED, None)
        );
        assert_eq!(
            MetaResult::Supercede(false).into_parts(),
            (META_RES::MRES_SUPERCEDE, Some(false))
        );
    }

    #[test]
    fn post_hook_results_map_to_metamod_codes() {
        assert_eq!(
            PostResult::<bool>::Ignored.into_parts(),
            (META_RES::MRES_IGNORED, None)
        );
        assert_eq!(
            PostResult::<bool>::Handled.into_parts(),
            (META_RES::MRES_HANDLED, None)
        );
        assert_eq!(
            PostResult::Override(true).into_parts(),
            (META_RES::MRES_OVERRIDE, Some(true))
        );
    }
}
