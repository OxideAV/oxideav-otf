//! CFF subroutine resolution helpers.
//!
//! Type 2 charstrings reference local and global subroutines via the
//! `callsubr` (op 10) and `callgsubr` (op 29) operators. The
//! subroutine number on the stack is *biased* — the actual INDEX
//! lookup uses `index = number + bias`, where the bias depends on
//! the INDEX size (TN5177 §4.7):
//!
//! ```text
//! count <  1240   →  bias =   107
//! count < 33900   →  bias = 1131
//! else            →  bias = 32768
//! ```
//!
//! This compaction puts the most-used subroutine numbers (the small
//! ones) closest to zero in the encoded charstring, saving bytes.
//! Get the bias wrong and every callsubr / callgsubr in the font
//! lands on the wrong subroutine — which is exactly the kind of
//! breakage the spec calls out as a common implementation mistake.

/// Compute the Type 2 subroutine bias for an INDEX of `count` subrs.
#[inline]
pub(crate) fn bias_for(count: u32) -> i32 {
    if count < 1240 {
        107
    } else if count < 33900 {
        1131
    } else {
        32768
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bias_thresholds() {
        // Below 1240.
        assert_eq!(bias_for(0), 107);
        assert_eq!(bias_for(1), 107);
        assert_eq!(bias_for(1239), 107);
        // Mid range.
        assert_eq!(bias_for(1240), 1131);
        assert_eq!(bias_for(33899), 1131);
        // High range — bias 32768 is rare in practice (would mean
        // a font with > 33900 subroutines) but the spec documents it.
        assert_eq!(bias_for(33900), 32768);
        assert_eq!(bias_for(100_000), 32768);
    }

    #[test]
    fn bias_examples() {
        // A subr index of "0" with 200 subrs → biased number is -107.
        // i.e. the encoded charstring's `0` after bias = 107 → 107 - 107 = 0.
        // So if the charstring encodes biased -107, real subr = 0.
        // We just assert the bias value is what real Type 2 charstrings
        // expect to add to retrieve subr 0.
        assert_eq!(bias_for(200), 107);
    }
}
