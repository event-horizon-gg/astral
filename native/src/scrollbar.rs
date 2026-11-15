/// Geometry shared by painting, track clicks and thumb dragging. Offsets use
/// GPUI's convention: zero at the top, negative while scrolling down.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub viewport: f32,
    pub maximum: f32,
    pub thumb_height: f32,
    pub thumb_top: f32,
}

impl Metrics {
    pub fn new(viewport: f32, maximum: f32, offset: f32) -> Option<Self> {
        if !viewport.is_finite()
            || !maximum.is_finite()
            || !offset.is_finite()
            || viewport <= 0.
            || maximum <= 0.
        {
            return None;
        }
        let thumb_height = (viewport * viewport / (viewport + maximum))
            .max(20.)
            .min(viewport);
        let thumb_top = (-offset / maximum).clamp(0., 1.) * (viewport - thumb_height);
        Some(Self {
            viewport,
            maximum,
            thumb_height,
            thumb_top,
        })
    }

    pub fn offset_for_top(self, top: f32) -> f32 {
        let travel = self.viewport - self.thumb_height;
        if travel <= 0. {
            return 0.;
        }
        -(top / travel).clamp(0., 1.) * self.maximum
    }

    pub fn contains_thumb(self, y: f32) -> bool {
        y >= self.thumb_top && y <= self.thumb_top + self.thumb_height
    }

    pub fn page_offset(self, y: f32, current: f32) -> f32 {
        (current
            + if y < self.thumb_top {
                self.viewport
            } else {
                -self.viewport
            })
        .clamp(-self.maximum, 0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_and_drag_reach_both_ends_of_a_long_page() {
        let top = Metrics::new(800., 2400., 0.).unwrap();
        assert_eq!(top.thumb_height, 200.);
        assert_eq!(top.thumb_top, 0.);
        assert_eq!(top.offset_for_top(600.), -2400.);
        assert_eq!(top.offset_for_top(-100.), 0.);
        assert_eq!(top.offset_for_top(1000.), -2400.);
        let middle = Metrics::new(800., 2400., -1200.).unwrap();
        assert_eq!(middle.thumb_top, 300.);
        assert!(middle.contains_thumb(400.));
        assert!(!middle.contains_thumb(200.));
        assert_eq!(middle.page_offset(200., -1200.), -400.);
        assert_eq!(middle.page_offset(700., -1200.), -2000.);
        assert_eq!(middle.page_offset(700., -2300.), -2400.);
        assert_eq!(Metrics::new(800., 2400., -4000.).unwrap().thumb_top, 600.);
        assert!(Metrics::new(800., 0., 0.).is_none());
        assert!(Metrics::new(f32::NAN, 2400., 0.).is_none());
    }
}
