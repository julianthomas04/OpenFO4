//! Assets the world needs but cannot use. A file that is absent is not broken: its stand-in
//! is drawn and a warning names it. A file that exists but cannot be read, parsed or uploaded
//! ends the game, and so does a load that never finishes.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokenAsset {
    Texture { path: String, reason: String },
}

impl fmt::Display for BrokenAsset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Texture { path, reason } => write!(f, "texture '{path}' is broken: {reason}"),
        }
    }
}

/// Every broken asset one frame ran into.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct BrokenAssets(pub Vec<BrokenAsset>);

impl fmt::Display for BrokenAssets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} broken asset(s): ", self.0.len())?;
        for (index, asset) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{asset}")?;
        }
        Ok(())
    }
}

/// Collects the broken assets found while a frame runs, for the frame to end the game with.
#[derive(Debug, Default)]
pub struct BrokenAssetSink(Vec<BrokenAsset>);

impl BrokenAssetSink {
    pub fn push(&mut self, asset: BrokenAsset) {
        self.0.push(asset);
    }

    pub fn extend(&mut self, assets: impl IntoIterator<Item = BrokenAsset>) {
        self.0.extend(assets);
    }

    /// `Err` with everything collected since the last call, `Ok` when nothing was.
    pub fn take(&mut self) -> Result<(), BrokenAssets> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(BrokenAssets(std::mem::take(&mut self.0)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(path: &str) -> BrokenAsset {
        BrokenAsset::Texture {
            path: path.to_owned(),
            reason: "invalid DDS".to_owned(),
        }
    }

    #[test]
    fn an_empty_sink_is_fine() {
        assert_eq!(BrokenAssetSink::default().take(), Ok(()));
    }

    #[test]
    fn the_sink_hands_over_everything_collected_once() {
        let mut sink = BrokenAssetSink::default();
        sink.push(texture("a.dds"));
        sink.extend([texture("b.dds")]);

        assert_eq!(
            sink.take(),
            Err(BrokenAssets(vec![texture("a.dds"), texture("b.dds")]))
        );
        assert_eq!(sink.take(), Ok(()));
    }

    #[test]
    fn the_report_names_every_broken_asset() {
        let report = BrokenAssets(vec![texture("a.dds"), texture("b.dds")]).to_string();
        assert_eq!(
            report,
            "2 broken asset(s): texture 'a.dds' is broken: invalid DDS; \
             texture 'b.dds' is broken: invalid DDS"
        );
    }
}
