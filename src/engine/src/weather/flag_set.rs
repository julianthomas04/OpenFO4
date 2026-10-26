/// Defines a flag enum and a typed set of its flags stored as the record's bit field. `from_bits`
/// rejects bits no flag names instead of dropping them.
macro_rules! flag_set {
    (
        $(#[$set_meta:meta])*
        $set:ident of $flag:ident {
            $($(#[$variant_meta:meta])* $variant:ident = $bit:literal),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $flag {
            $($(#[$variant_meta])* $variant),+
        }

        impl $flag {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub const fn bit(self) -> u8 {
                match self {
                    $(Self::$variant => $bit),+
                }
            }
        }

        $(#[$set_meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
        pub struct $set(u8);

        impl $set {
            const KNOWN_BITS: u8 = 0 $(| $bit)+;

            /// `None` when `bits` has a bit that no flag names.
            #[allow(clippy::bad_bit_mask, reason = "a set naming every bit has no unknown bits")]
            pub fn from_bits(bits: u32) -> Option<Self> {
                let bits = u8::try_from(bits).ok()?;
                (bits & !Self::KNOWN_BITS == 0).then_some(Self(bits))
            }

            pub const fn bits(self) -> u8 {
                self.0
            }

            pub const fn contains(self, flag: $flag) -> bool {
                self.0 & flag.bit() != 0
            }
        }
    };
}

pub(super) use flag_set;
