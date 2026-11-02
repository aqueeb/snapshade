//! Snapshot of the connected displays.

use serde::{Deserialize, Serialize};

use crate::geometry::{DisplayId, DisplayInfo, LogicalPoint};

/// All connected displays at one instant.
///
/// Ordering invariant: the primary display first, then the rest in ascending
/// [`DisplayId`]. Hit-testing a global logical point uses the half-open rule
/// `x ∈ [x0, x0 + w)` and, where logical rects overlap (possible on Windows
/// mixed-DPI), the first display in this order wins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayTopology {
    pub displays: Vec<DisplayInfo>,
}

impl DisplayTopology {
    /// Build a topology, enforcing the ordering invariant: primary first, then
    /// ascending id.
    pub fn new(mut displays: Vec<DisplayInfo>) -> Self {
        // Stable sort: primary first (false < true, so negate), then ascending id.
        displays.sort_by_key(|d| (!d.primary, d.id));
        Self { displays }
    }

    /// The display with `id`, if connected.
    pub fn get(&self, id: DisplayId) -> Option<&DisplayInfo> {
        self.displays.iter().find(|d| d.id == id)
    }

    /// The display whose logical rect contains `p` (half-open on each axis);
    /// the first match in topology order wins, so a point on a shared edge
    /// belongs to exactly one display and a point in a gap belongs to none.
    pub fn hit_test(&self, p: LogicalPoint) -> Option<DisplayId> {
        self.displays
            .iter()
            .find(|d| {
                let r = &d.logical;
                p.x >= r.x && p.x < r.x + r.w && p.y >= r.y && p.y < r.y + r.h
            })
            .map(|d| d.id)
    }

    pub fn is_empty(&self) -> bool {
        self.displays.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayTopology;
    use crate::geometry::{DisplayInfo, LogicalRect, PhysicalSize};

    fn display(id: u32, primary: bool) -> DisplayInfo {
        DisplayInfo {
            id,
            logical: LogicalRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
            scale: 1.0,
            physical: PhysicalSize { w: 100, h: 100 },
            primary,
        }
    }

    #[test]
    fn new_orders_primary_first_then_ascending_id() {
        let topo =
            DisplayTopology::new(vec![display(3, false), display(2, false), display(1, true)]);
        let ids: Vec<u32> = topo.displays.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);

        let topo = DisplayTopology::new(vec![display(2, false), display(1, true)]);
        let ids: Vec<u32> = topo.displays.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn primary_leads_even_with_a_higher_id() {
        let topo = DisplayTopology::new(vec![display(1, false), display(7, true)]);
        let ids: Vec<u32> = topo.displays.iter().map(|d| d.id).collect();
        assert_eq!(ids, vec![7, 1]);
        assert_eq!(topo.get(7).map(|d| d.id), Some(7));
        assert_eq!(topo.get(2), None);
    }

    #[test]
    fn is_empty_for_no_displays() {
        assert!(DisplayTopology::new(vec![]).is_empty());
        assert!(!DisplayTopology::new(vec![display(1, true)]).is_empty());
    }
}
