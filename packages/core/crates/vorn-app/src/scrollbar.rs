//! A terminal's overlay scrollbar: where its thumb sits over the history,
//! what a drag or a click in the track asks for, and when it shows. It
//! appears while the terminal scrolls and under the pointer, then fades.

use std::time::Duration;

/// The thumb never gets shorter than this, logical pixels.
pub const MIN_THUMB: f32 = 24.0;
/// How long the bar stays after the last scroll.
pub const SHOW: Duration = Duration::from_millis(900);
/// Its fade out after that.
pub const FADE: Duration = Duration::from_millis(250);

/// The thumb in a track, logical pixels from the track's top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thumb {
    pub top: f32,
    pub h: f32,
}

/// The thumb of a `track` tall bar over `rows` on screen below `history`
/// rows, `back` rows scrolled up; `None` with no history to scroll.
pub fn thumb(track: f32, rows: u16, history: u64, back: u64) -> Option<Thumb> {
    if history == 0 || track < 1.0 {
        return None;
    }
    let total = history as f32 + f32::from(rows.max(1));
    let h = (track * f32::from(rows.max(1)) / total).clamp(MIN_THUMB.min(track), track);
    let along = 1.0 - back.min(history) as f32 / history as f32;
    Some(Thumb {
        top: (track - h) * along,
        h,
    })
}

/// The rows back a thumb moved to `top` shows.
pub fn back_at(track: f32, rows: u16, history: u64, top: f32) -> u64 {
    let Some(t) = thumb(track, rows, history, 0) else {
        return 0;
    };
    let free = track - t.h;
    if free <= 0.0 {
        return 0;
    }
    let along = (top / free).clamp(0.0, 1.0);
    ((1.0 - along) * history as f32).round() as u64
}

/// Rows a click in the track pages by: a screen less one row, kept for context.
pub fn page(rows: u16) -> u64 {
    u64::from(rows.saturating_sub(1).max(1))
}

/// How opaque the bar is `since` the last scroll; full under the pointer or a drag.
pub fn alpha(since: Option<Duration>, held: bool) -> f32 {
    if held {
        return 1.0;
    }
    match since {
        Some(d) if d <= SHOW => 1.0,
        Some(d) if d < SHOW + FADE => 1.0 - (d - SHOW).as_secs_f32() / FADE.as_secs_f32(),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_is_the_screen_share_of_the_track_and_sits_by_position() {
        let at_bottom = thumb(400.0, 40, 360, 0).unwrap();
        assert_eq!(
            at_bottom,
            Thumb {
                top: 360.0,
                h: 40.0
            }
        );
        let at_top = thumb(400.0, 40, 360, 360).unwrap();
        assert_eq!(at_top.top, 0.0);
        let half = thumb(400.0, 40, 360, 180).unwrap();
        assert_eq!(half.top, 180.0);
        assert_eq!(thumb(400.0, 40, 0, 0), None, "nothing to scroll");
        assert_eq!(
            thumb(400.0, 40, 9, 99).unwrap().top,
            0.0,
            "back past the history"
        );
    }

    #[test]
    fn a_long_history_keeps_a_grabbable_thumb() {
        let t = thumb(300.0, 30, 100_000, 50_000).unwrap();
        assert_eq!(t.h, MIN_THUMB);
        assert!((t.top - (300.0 - MIN_THUMB) / 2.0).abs() < 0.01);
        assert_eq!(
            thumb(10.0, 30, 100, 0).unwrap().h,
            10.0,
            "never taller than its track"
        );
    }

    #[test]
    fn dragging_the_thumb_maps_back_to_rows() {
        for back in [0, 1, 90, 180, 359, 360] {
            let t = thumb(400.0, 40, 360, back).unwrap();
            assert_eq!(back_at(400.0, 40, 360, t.top), back, "round trip at {back}");
        }
        assert_eq!(
            back_at(400.0, 40, 360, -50.0),
            360,
            "above the track is the oldest line"
        );
        assert_eq!(
            back_at(400.0, 40, 360, 999.0),
            0,
            "below is the live screen"
        );
        assert_eq!(back_at(400.0, 40, 0, 10.0), 0);
    }

    #[test]
    fn the_bar_shows_after_a_scroll_then_fades() {
        assert_eq!(alpha(None, false), 0.0);
        assert_eq!(alpha(None, true), 1.0, "under the pointer");
        assert_eq!(alpha(Some(Duration::from_millis(100)), false), 1.0);
        let mid = alpha(Some(SHOW + FADE / 2), false);
        assert!((mid - 0.5).abs() < 0.01, "{mid}");
        assert_eq!(alpha(Some(SHOW + FADE), false), 0.0);
        assert_eq!(page(40), 39);
        assert_eq!(page(1), 1);
    }
}
