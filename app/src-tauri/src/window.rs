//! The window's first size: the size the app is made for, shrunk to fit the screen it opens on.

/// The size the window opens at on a large enough screen, in logical pixels.
pub const WANT: (f64, f64) = (1200.0, 800.0);
/// The smallest size the window can be made, in logical pixels, unless the screen is smaller.
pub const MIN: (f64, f64) = (960.0, 640.0);

/// Where and how large the window opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Inner size, logical pixels.
    pub size: (f64, f64),
    /// Smallest inner size, logical pixels.
    pub min: (f64, f64),
    /// Top-left corner of the outer window, physical pixels.
    pub position: (i32, i32),
}

/// The window's size and position for a screen whose work area (the screen without the taskbar
/// or the menu bar and Dock) is `work` (x, y, width, height in physical pixels) at `scale`
/// physical pixels per logical one, with a frame (title bar and borders) of `frame` physical
/// pixels around the inner size: [`WANT`] or less so the whole window fits, centred in the
/// work area. The smallest size is [`MIN`], lowered to the fitted size on a smaller screen.
pub fn fit(work: (i32, i32, u32, u32), frame: (u32, u32), scale: f64) -> Fit {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let (wx, wy, ww, wh) = work;
    let room = |total: u32, frame: u32| (f64::from(total.saturating_sub(frame)) / scale).floor();
    let size = (WANT.0.min(room(ww, frame.0)), WANT.1.min(room(wh, frame.1)));
    let min = (MIN.0.min(size.0), MIN.1.min(size.1));
    let outer = |inner: f64, frame: u32| (inner * scale).round() as i64 + i64::from(frame);
    let centre = |start: i32, total: u32, outer: i64| {
        (i64::from(start) + ((i64::from(total) - outer) / 2).max(0)) as i32
    };
    Fit {
        size,
        min,
        position: (
            centre(wx, ww, outer(size.0, frame.0)),
            centre(wy, wh, outer(size.1, frame.1)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_screen_gets_the_usual_size() {
        let f = fit((0, 0, 2560, 1400), (0, 28), 1.0);
        assert_eq!(f.size, WANT);
        assert_eq!(f.min, MIN);
        assert_eq!(f.position, (680, 286));
    }

    #[test]
    fn a_small_screen_gets_a_window_that_fits() {
        // 1366x768 with a 48 px taskbar and a 31 px title bar and 8 px borders.
        let f = fit((0, 0, 1366, 720), (16, 39), 1.0);
        assert_eq!(f.size, (1200.0, 681.0));
        assert_eq!(f.min, (960.0, 640.0));
        assert!(f.position.1 >= 0 && f.position.1 + 681 + 39 <= 720, "{f:?}");
        // 1024x768: narrower than the usual minimum, which is lowered.
        let f = fit((0, 0, 1024, 728), (16, 39), 1.0);
        assert_eq!(f.size, (1008.0, 689.0));
        assert_eq!(f.min, (960.0, 640.0));
        let f = fit((0, 0, 800, 600), (16, 39), 1.0);
        assert_eq!(f.size, (784.0, 561.0));
        assert_eq!(f.min, (784.0, 561.0));
        assert_eq!(f.position, (0, 0));
    }

    #[test]
    fn scale_and_offset_are_kept() {
        // A second screen to the right, at 150 %.
        let f = fit((1920, 0, 1920, 1032), (0, 0), 1.5);
        assert_eq!(f.size, (1200.0, 688.0));
        assert_eq!(f.position, (1920 + (1920 - 1800) / 2, 0));
        assert_eq!(fit((0, 0, 1920, 1080), (0, 0), f64::NAN).size, WANT);
    }
}
