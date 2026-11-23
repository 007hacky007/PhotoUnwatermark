//! Tiling helpers shared by the inpainting engine.

/// Tile edge length in pixels.
pub const TILE: u32 = 512;
/// Step between consecutive tile starts in pixels.
pub const STRIDE: u32 = 384;

/// Start offsets of tiles covering `length` pixels.
///
/// Starts advance by `stride`; the last start is clamped to `length - tile`
/// so the final tile ends exactly at the edge. A single `0` is returned when
/// `length <= tile`.
pub fn tile_starts(length: u32, tile: u32, stride: u32) -> Vec<u32> {
    if length <= tile {
        return vec![0];
    }
    let last = length - tile;
    let mut starts = Vec::new();
    let mut pos = 0;
    while pos < last {
        starts.push(pos);
        pos += stride;
    }
    starts.push(last);
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_exactly() {
        assert_eq!(tile_starts(1000, 512, 384), vec![0, 384, 488]);
    }

    #[test]
    fn shorter_than_tile() {
        assert_eq!(tile_starts(300, 512, 384), vec![0]);
    }

    #[test]
    fn exact_fit() {
        assert_eq!(tile_starts(512, 512, 384), vec![0]);
    }
}
