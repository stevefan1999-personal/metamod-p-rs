use crate::sys::META_RES;

/// What a hook tells metamod about the call it intercepted.
///
/// Metamod-P and metamod-svencoop keep a return value only from
/// `Supercede` in a pre hook and from `Override` in a post hook. An
/// `Override` from a pre hook marks the call as overridden without keeping
/// the value, so the caller gets zero; use `Supercede` in pre hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaResult<T> {
    /// The hook did nothing; the real function runs.
    Ignored,
    /// The hook did something; the real function still runs.
    Handled,
    /// The real function runs, but the caller gets this value. Post hooks
    /// only.
    Override(T),
    /// The real function is skipped and the caller gets this value. Pre
    /// hooks only.
    Supercede(T),
}

impl<T> MetaResult<T> {
    /// The `META_RES` code for metamod and the value to return, if any.
    pub(crate) fn into_parts(self) -> (META_RES, Option<T>) {
        match self {
            Self::Ignored => (META_RES::MRES_IGNORED, None),
            Self::Handled => (META_RES::MRES_HANDLED, None),
            Self::Override(value) => (META_RES::MRES_OVERRIDE, Some(value)),
            Self::Supercede(value) => (META_RES::MRES_SUPERCEDE, Some(value)),
        }
    }
}

/// Return values that a post hook can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Returned<T> {
    /// The value the call produced: the real function's result, or the
    /// value of a pre hook that superceded it.
    pub original:   T,
    /// The value an earlier post hook set with [`MetaResult::Override`].
    pub overridden: Option<T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_map_to_metamod_codes() {
        assert_eq!(
            MetaResult::<bool>::Ignored.into_parts(),
            (META_RES::MRES_IGNORED, None)
        );
        assert_eq!(
            MetaResult::<bool>::Handled.into_parts(),
            (META_RES::MRES_HANDLED, None)
        );
        assert_eq!(
            MetaResult::Override(true).into_parts(),
            (META_RES::MRES_OVERRIDE, Some(true))
        );
        assert_eq!(
            MetaResult::Supercede(false).into_parts(),
            (META_RES::MRES_SUPERCEDE, Some(false))
        );
    }
}
